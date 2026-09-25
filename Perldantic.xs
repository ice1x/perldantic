/*
 * Native parts of the Perl side of Perldantic (docs/PLAN.md, stage 00057).
 *
 * Perldantic::XS::encode($value, $fallback) writes a Perl value as wire JSON, exactly as the
 * pure-Perl Perldantic::Wire::_emit does, for plain data: undef, booleans, numbers, strings,
 * arrays and hashes. Everything else (objects, code references, hashes with keys starting
 * with `$`) is written by calling $fallback with the value, which returns its wire JSON.
 * Perldantic::XS::encode_pairs([$key, $value, ...], $fallback) writes a JSON object with the
 * keys in the given order (the core keeps key order).
 */
#define PERL_NO_GET_CONTEXT
#include "EXTERN.h"
#include "perl.h"
#include "XSUB.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* The Perl side's registries the decoder reads on every call, looked up once per interpreter
   (they are package hashes, emptied but never replaced). */
#define MY_CXT_KEY "Perldantic::XS::_guts" XS_VERSION
typedef struct {
    HV *bless; /* %Perldantic::Wire::BLESS */
    HV *lazy;  /* %Perldantic::Wire::LAZY */
    HV *enums; /* %Perldantic::Enum::CLASSES */
} my_cxt_t;
START_MY_CXT

static void cxt_fetch(pTHX_ my_cxt_t *cxt)
{
    cxt->bless = get_hv("Perldantic::Wire::BLESS", GV_ADD);
    cxt->lazy = get_hv("Perldantic::Wire::LAZY", GV_ADD);
    cxt->enums = get_hv("Perldantic::Enum::CLASSES", GV_ADD);
}

/* How deep plain data may nest before the fallback (which has no such limit) takes over. */
#define MAX_DEPTH 512

/* What one encode call needs: the fallback for values it does not write itself, and what it
   takes to write model objects natively (see emit_model). */
typedef struct {
    SV *fallback;
    HV *direct;   /* %Perldantic::Wire::DIRECT: class => [field names] */
    HV *state;    /* %Perldantic::Model::STATE, a field hash keyed by object address */
    HV *lazy_dump; /* %Perldantic::Wire::LAZY_DUMP: class => [fields left out unless set] */
    int tracking; /* $Perldantic::Model::TRACK_OBJECTS: objects then carry tokens */
    int guarded;  /* Rust frames are on the stack: a dying fallback must not unwind through them */
    SV *error;    /* the first exception a guarded fallback raised (owned) */
} encoder;

static void emit(pTHX_ SV *out, SV *value, encoder *enc, int depth);

/* Lazy objects (task 00081): a blessed, empty hash with ext magic holding the handle of a model
   the core keeps (ffi/src/lazy.rs). The magic releases the handle with the object; a thread
   cloning the object gets its own handle. Perldantic::Model::_realize reads the fields in. */
typedef void (*pd_buffer_free_fn)(unsigned char *buffer, size_t len);
typedef unsigned char *(*pd_lazy_contents_fn)(U64 handle, size_t *len);
typedef U64 (*pd_lazy_clone_fn)(U64 handle);
typedef void (*pd_lazy_release_fn)(U64 handle);

static pd_buffer_free_fn core_buffer_free = NULL;
static pd_lazy_contents_fn core_lazy_contents = NULL;
static pd_lazy_clone_fn core_lazy_clone = NULL;
static pd_lazy_release_fn core_lazy_release = NULL;

static int lazy_free(pTHX_ SV *sv, MAGIC *mg)
{
    UV handle = PTR2UV(mg->mg_ptr);
    PERL_UNUSED_ARG(sv);
    if (handle && core_lazy_release) core_lazy_release((U64)handle);
    mg->mg_ptr = NULL;
    return 0;
}

#ifdef USE_ITHREADS
static int lazy_dup(pTHX_ MAGIC *mg, CLONE_PARAMS *param)
{
    UV handle = PTR2UV(mg->mg_ptr);
    PERL_UNUSED_ARG(param);
    mg->mg_ptr = handle && core_lazy_clone ? INT2PTR(char *, (UV)core_lazy_clone((U64)handle)) : NULL;
    return 0;
}
#endif

static MGVTBL lazy_vtbl = {
    NULL, NULL, NULL, NULL, lazy_free, NULL,
#ifdef USE_ITHREADS
    lazy_dup,
#else
    NULL,
#endif
    NULL
};

/* The lazy magic of a hash, if it is a lazy object not read yet. */
static MAGIC *lazy_magic(pTHX_ SV *target)
{
    MAGIC *mg;
    if (!SvRMAGICAL(target)) return NULL;
    mg = mg_findext(target, PERL_MAGIC_ext, &lazy_vtbl);
    return mg && mg->mg_ptr ? mg : NULL;
}

/* JSON text of a UTF-8 byte string: quotes, backslashes and control characters escaped. */
static void emit_string(pTHX_ SV *out, const char *text, STRLEN len)
{
    static const char hex[] = "0123456789abcdef";
    STRLEN i, start = 0;
    sv_catpvn(out, "\"", 1);
    for (i = 0; i < len; i++) {
        unsigned char c = (unsigned char)text[i];
        const char *escape = NULL;
        char unicode[7];
        if (c == '"') escape = "\\\"";
        else if (c == '\\') escape = "\\\\";
        else if (c == '\n') escape = "\\n";
        else if (c == '\r') escape = "\\r";
        else if (c == '\t') escape = "\\t";
        else if (c == '\b') escape = "\\b";
        else if (c == '\f') escape = "\\f";
        else if (c < 0x20) {
            unicode[0] = '\\'; unicode[1] = 'u'; unicode[2] = '0'; unicode[3] = '0';
            unicode[4] = hex[c >> 4]; unicode[5] = hex[c & 15]; unicode[6] = '\0';
            escape = unicode;
        }
        if (escape) {
            if (i > start) sv_catpvn(out, text + start, i - start);
            sv_catpv(out, escape);
            start = i + 1;
        }
    }
    if (len > start) sv_catpvn(out, text + start, len - start);
    sv_catpvn(out, "\"", 1);
}

/* A Perl string as JSON: character strings as UTF-8, byte strings upgraded as Perl does. */
static void emit_sv_string(pTHX_ SV *out, SV *value)
{
    STRLEN len, i;
    const char *text = SvPV_const(value, len);
    if (!SvUTF8(value)) {
        /* bytes above 0x7f are Latin-1 characters: write them as UTF-8 (ASCII already is) */
        for (i = 0; i < len; i++) if ((unsigned char)text[i] & 0x80) break;
        if (i < len) {
            SV *copy = sv_2mortal(newSVsv(value));
            text = SvPVutf8(copy, len);
        }
    }
    emit_string(aTHX_ out, text, len);
}

/* The shortest decimal form that reads back as the same double, with a fraction or an
   exponent so that the core sees a float (as Perldantic::Wire::_float). */
static void emit_float(pTHX_ SV *out, NV value)
{
    char text[40];
    int digits, from;
    if (value != value) { sv_catpv(out, "{\"$float\":\"nan\"}"); return; }
    if (isinf(value)) {
        sv_catpv(out, value > 0 ? "{\"$float\":\"inf\"}" : "{\"$float\":\"-inf\"}");
        return;
    }
    from = (value != 0 && fabs(value) < 2.2250738585072014e-308) ? 1 : 15;
    for (digits = from; digits <= 17; digits++) {
        snprintf(text, sizeof text, "%.*g", digits, (double)value);
        if (strtod(text, NULL) == (double)value) break;
    }
    sv_catpv(out, text);
    if (!strpbrk(text, ".eE")) sv_catpvn(out, ".0", 2);
}

/* What the fallback writes for a value. */
static void emit_fallback(pTHX_ SV *out, SV *value, SV *fallback)
{
    dSP;
    int count;
    SV *json;
    ENTER;
    SAVETMPS;
    PUSHMARK(SP);
    XPUSHs(value);
    PUTBACK;
    count = call_sv(fallback, G_SCALAR);
    SPAGAIN;
    if (count != 1) croak("Perldantic::XS::encode: the fallback returned no value");
    json = POPs;
    sv_catsv(out, json);
    PUTBACK;
    FREETMPS;
    LEAVE;
}

static int compare_keys(const void *a, const void *b)
{
    dTHX;
    return sv_cmp(*(SV *const *)a, *(SV *const *)b);
}

static void emit_hash(pTHX_ SV *out, SV *ref, HV *hash, encoder *enc, int depth)
{
    I32 count = hv_iterinit(hash);
    SV **keys;
    HE *entry;
    I32 i = 0;
    if (count == 0) { sv_catpvn(out, "{}", 2); return; }
    Newx(keys, count, SV *);
    SAVEFREEPV(keys);
    while ((entry = hv_iternext(hash)) && i < count) {
        SV *key = hv_iterkeysv(entry);
        STRLEN len;
        const char *text = SvPV_const(key, len);
        if (len > 0 && text[0] == '$') {
            /* such keys go as $dict pairs */
            emit_fallback(aTHX_ out, ref, enc->fallback);
            return;
        }
        keys[i++] = key;
    }
    count = i;
    qsort(keys, count, sizeof(SV *), compare_keys);
    sv_catpvn(out, "{", 1);
    for (i = 0; i < count; i++) {
        HE *found = hv_fetch_ent(hash, keys[i], 0, 0);
        if (i > 0) sv_catpvn(out, ",", 1);
        emit_sv_string(aTHX_ out, keys[i]);
        sv_catpvn(out, ":", 1);
        emit(aTHX_ out, found ? HeVAL(found) : &PL_sv_undef, enc, depth + 1);
    }
    sv_catpvn(out, "}", 1);
}

/* A Perldantic model object as {"$model": {"class", "fields"}}, when it can be written without
   Perl: its class is registered with its field names, it has no state (so it has set exactly
   the fields it holds, which the core assumes when fields_set is left out) and no call tracks
   objects. Returns 0 when the object must go through Perl. */
static int emit_model(pTHX_ SV *out, SV *target, encoder *enc, int depth)
{
    HV *stash = SvSTASH(target);
    const char *class = HvNAME(stash);
    SV **names;
    AV *list;
    SSize_t last, i;
    int first = 1;
    char address[32];
    int length;
    if (enc->tracking || !enc->direct || !class || lazy_magic(aTHX_ target)) return 0;
    names = hv_fetch(enc->direct, class,
        HvNAMEUTF8(stash) ? -(I32)HvNAMELEN(stash) : (I32)HvNAMELEN(stash), 0);
    if (!names || !SvROK(*names) || SvTYPE(SvRV(*names)) != SVt_PVAV) return 0;
    if (enc->state) {
        length = snprintf(address, sizeof address, "%" UVuf, PTR2UV(target));
        if (hv_exists(enc->state, address, length)) return 0;
    }
    list = (AV *)SvRV(*names);
    last = av_len(list);
    sv_catpv(out, "{\"$model\":{\"class\":");
    emit_string(aTHX_ out, class, HvNAMELEN(stash));
    sv_catpv(out, ",\"fields\":{");
    for (i = 0; i <= last; i++) {
        SV **name = av_fetch(list, i, 0);
        HE *field;
        if (!name) continue;
        field = hv_fetch_ent((HV *)target, *name, 0, 0);
        if (!field) continue;
        if (!first) sv_catpvn(out, ",", 1);
        first = 0;
        emit_sv_string(aTHX_ out, *name);
        sv_catpvn(out, ":", 1);
        emit(aTHX_ out, HeVAL(field), enc, depth + 1);
    }
    sv_catpvn(out, "}}}", 3);
    return 1;
}

static void emit(pTHX_ SV *out, SV *value, encoder *enc, int depth)
{
    SvGETMAGIC(value);
    if (depth > MAX_DEPTH) { emit_fallback(aTHX_ out, value, enc->fallback); return; }
    if (SvROK(value)) {
        SV *target = SvRV(value);
        if (SvOBJECT(target)) {
            if (SvTYPE(target) == SVt_PVHV && emit_model(aTHX_ out, target, enc, depth)) return;
            emit_fallback(aTHX_ out, value, enc->fallback);
            return;
        }
        /* tied arrays and hashes: Perl sees their elements, a C walk would not */
        if (SvRMAGICAL(target)) { emit_fallback(aTHX_ out, value, enc->fallback); return; }
        if (SvTYPE(target) == SVt_PVAV) {
            AV *array = (AV *)target;
            SSize_t last = av_len(array), i;
            sv_catpvn(out, "[", 1);
            for (i = 0; i <= last; i++) {
                SV **item = av_fetch(array, i, 0);
                if (i > 0) sv_catpvn(out, ",", 1);
                emit(aTHX_ out, item ? *item : &PL_sv_undef, enc, depth + 1);
            }
            sv_catpvn(out, "]", 1);
            return;
        }
        if (SvTYPE(target) == SVt_PVHV) {
            ENTER;
            emit_hash(aTHX_ out, value, (HV *)target, enc, depth);
            LEAVE;
            return;
        }
        /* code references and anything else: the fallback knows (or refuses) */
        emit_fallback(aTHX_ out, value, enc->fallback);
        return;
    }
    if (!SvOK(value)) { sv_catpvn(out, "null", 4); return; }
#ifdef SvIsBOOL
    if (SvIsBOOL(value)) {
        if (SvTRUE_nomg(value)) sv_catpvn(out, "true", 4); else sv_catpvn(out, "false", 5);
        return;
    }
#endif
    /* numbers that were never strings (builtin::created_as_number) */
    if ((SvIOK(value) || SvNOK(value)) && !SvPOK(value)) {
        if (SvIOK(value)) {
            if (SvIsUV(value)) sv_catpvf(out, "%" UVuf, SvUV_nomg(value));
            else sv_catpvf(out, "%" IVdf, SvIV_nomg(value));
        } else {
            emit_float(aTHX_ out, SvNV_nomg(value));
        }
        return;
    }
    emit_sv_string(aTHX_ out, value);
}

/* A JSON object with the given keys in the given order: [key, value, key, value, ...]. */
static void emit_pairs(pTHX_ SV *out, AV *pairs, encoder *enc)
{
    SSize_t last = av_len(pairs), i;
    sv_catpvn(out, "{", 1);
    for (i = 0; i + 1 <= last; i += 2) {
        SV **key = av_fetch(pairs, i, 0);
        SV **value = av_fetch(pairs, i + 1, 0);
        if (i > 0) sv_catpvn(out, ",", 1);
        emit_sv_string(aTHX_ out, key ? *key : &PL_sv_no);
        sv_catpvn(out, ":", 1);
        emit(aTHX_ out, value ? *value : &PL_sv_undef, enc, 1);
    }
    sv_catpvn(out, "}", 1);
}

/*
 * The binary wire format (ffi/src/binary.rs): what the bulk of the data crosses the boundary
 * in, written straight from Perl values and read straight into them.
 */
enum {
    B_NONE = 0, B_TRUE = 1, B_FALSE = 2, B_INT = 3, B_FLOAT = 4, B_STR = 5, B_LIST = 6,
    B_DICT = 7, B_JSON = 8, B_MODEL = 9, B_MODEL_FULL = 10, B_BYTES = 11, B_LAZY = 12,
    B_ENUM = 13
};

/* The registry entry of a Perl enum class (Perldantic::Enum), or NULL. */
static HV *enum_class(pTHX_ SV *class)
{
    dMY_CXT;
    HE *entry = MY_CXT.enums ? hv_fetch_ent(MY_CXT.enums, class, 0, 0) : NULL;
    if (!entry || !SvROK(HeVAL(entry)) || SvTYPE(SvRV(HeVAL(entry))) != SVt_PVHV) return NULL;
    return (HV *)SvRV(HeVAL(entry));
}

static void bput_len(pTHX_ SV *out, STRLEN len)
{
    unsigned char raw[4];
    U32 n = (U32)len;
    if ((STRLEN)n != len) croak("Perldantic::XS::encode_binary: a value is larger than 4 GiB");
    raw[0] = n & 0xff; raw[1] = (n >> 8) & 0xff; raw[2] = (n >> 16) & 0xff; raw[3] = (n >> 24) & 0xff;
    sv_catpvn(out, (const char *)raw, 4);
}

static void bput_tag(pTHX_ SV *out, unsigned char tag)
{
    sv_catpvn(out, (const char *)&tag, 1);
}

static void bput_bytes(pTHX_ SV *out, const char *bytes, STRLEN len)
{
    bput_len(aTHX_ out, len);
    sv_catpvn(out, bytes, len);
}

static void bput_u64(pTHX_ SV *out, U64 bits)
{
    unsigned char raw[8];
    int i;
    for (i = 0; i < 8; i++) raw[i] = (bits >> (8 * i)) & 0xff;
    sv_catpvn(out, (const char *)raw, 8);
}

/* A Perl string as UTF-8 bytes: byte strings are Latin-1 characters, as Perl upgrades them. */
static void bput_sv_string(pTHX_ SV *out, SV *value)
{
    STRLEN len, i;
    const char *text;
    if (SvUTF8(value)) {
        text = SvPV_const(value, len);
    } else {
        /* ASCII is UTF-8 already: only Latin-1 bytes need the upgraded copy */
        text = SvPV_const(value, len);
        for (i = 0; i < len; i++) if ((unsigned char)text[i] & 0x80) break;
        if (i == len) {
            bput_bytes(aTHX_ out, text, len);
            return;
        }
        SV *copy = sv_2mortal(newSVsv(value));
        text = SvPVutf8(copy, len);
    }
    bput_bytes(aTHX_ out, text, len);
}

/* What the fallback writes for a value, as a JSON node. */
static void bput_fallback(pTHX_ SV *out, SV *value, encoder *enc)
{
    SV *json = sv_2mortal(newSVpvn("", 0));
    if (enc->guarded) {
        /* call the fallback under G_EVAL: an exception is kept for the caller to raise once
           the core has returned, and the value written as undef meanwhile */
        dSP;
        int count;
        if (enc->error) { bput_tag(aTHX_ out, B_NONE); return; }
        ENTER;
        SAVETMPS;
        PUSHMARK(SP);
        XPUSHs(value);
        PUTBACK;
        count = call_sv(enc->fallback, G_SCALAR | G_EVAL);
        SPAGAIN;
        if (SvTRUE(ERRSV) || count != 1) {
            if (count == 1) (void)POPs;
            enc->error = newSVsv(SvTRUE(ERRSV) ? ERRSV : sv_2mortal(newSVpvs("the fallback returned no value")));
            PUTBACK;
            FREETMPS;
            LEAVE;
            bput_tag(aTHX_ out, B_NONE);
            return;
        }
        sv_catsv(json, POPs);
        PUTBACK;
        FREETMPS;
        LEAVE;
        bput_tag(aTHX_ out, B_JSON);
        bput_bytes(aTHX_ out, SvPVX(json), SvCUR(json));
        return;
    }
    emit_fallback(aTHX_ json, value, enc->fallback);
    bput_tag(aTHX_ out, B_JSON);
    bput_bytes(aTHX_ out, SvPVX(json), SvCUR(json));
}

static void bemit(pTHX_ SV *out, SV *value, encoder *enc, int depth);

/* A member of a Perl enum class as an enum node, with the class's sub type as its mixin (see
   Perldantic::Enum::_perldantic_wire). Returns 0 for objects of other classes. */
static int bemit_enum(pTHX_ SV *out, SV *target, encoder *enc, int depth)
{
    HV *stash = SvSTASH(target);
    const char *class = HvNAME(stash);
    SV *class_sv;
    HV *entry;
    SV **name, **value, **sub_type;
    if (!class) return 0;
    class_sv = sv_2mortal(newSVpvn_flags(class, HvNAMELEN(stash), HvNAMEUTF8(stash) ? SVf_UTF8 : 0));
    if (!(entry = enum_class(aTHX_ class_sv))) return 0;
    name = hv_fetchs((HV *)target, "name", 0);
    value = hv_fetchs((HV *)target, "value", 0);
    if (!name || !value) return 0;
    sub_type = hv_fetchs(entry, "sub_type", 0);
    bput_tag(aTHX_ out, B_ENUM);
    bput_bytes(aTHX_ out, class, HvNAMELEN(stash));
    bput_sv_string(aTHX_ out, *name);
    if (sub_type && SvOK(*sub_type)) bput_sv_string(aTHX_ out, *sub_type);
    else bput_len(aTHX_ out, 0);
    bput_tag(aTHX_ out, 1); /* a member reads as its value */
    bemit(aTHX_ out, *value, enc, depth + 1);
    return 1;
}

/* A model object as a model node, under the conditions of emit_model. */
static int bemit_model(pTHX_ SV *out, SV *target, encoder *enc, int depth)
{
    HV *stash = SvSTASH(target);
    const char *class = HvNAME(stash);
    SV **names;
    AV *list;
    SSize_t last, i;
    U32 count = 0;
    STRLEN count_at;
    char address[32];
    int length;
    MAGIC *lazy;
    if (enc->tracking || !class) return 0;
    if ((lazy = lazy_magic(aTHX_ target)) != NULL) {
        /* the core's own model, when it is what the object would hold: not when fields were
           stored in the hash directly, or the class fills fields in Perl (then Perl reads the
           object in first) */
        SV **drop = NULL;
        AV *names;
        SSize_t n;
        if (!HvUSEDKEYS((HV *)target) && enc->lazy_dump)
            drop = hv_fetch(enc->lazy_dump, class,
                HvNAMEUTF8(stash) ? -(I32)HvNAMELEN(stash) : (I32)HvNAMELEN(stash), 0);
        if (!drop || !SvROK(*drop) || SvTYPE(SvRV(*drop)) != SVt_PVAV) {
            /* read in by Perl, then written as any object (its own lazy objects stay lazy);
               not while Rust frames are on the stack, where the fallback guards errors */
            dSP;
            if (enc->guarded) return 0;
            ENTER;
            SAVETMPS;
            PUSHMARK(SP);
            XPUSHs(sv_2mortal(newRV_inc(target)));
            PUTBACK;
            call_pv("Perldantic::Model::_realize", G_DISCARD);
            FREETMPS;
            LEAVE;
            return lazy_magic(aTHX_ target) ? 0 : bemit_model(aTHX_ out, target, enc, depth);
        }
        names = (AV *)SvRV(*drop);
        bput_tag(aTHX_ out, B_LAZY);
        bput_bytes(aTHX_ out, class, HvNAMELEN(stash));
        bput_u64(aTHX_ out, (U64)PTR2UV(lazy->mg_ptr));
        bput_len(aTHX_ out, (STRLEN)(av_len(names) + 1));
        for (n = 0; n <= av_len(names); n++) {
            SV **name = av_fetch(names, n, 0);
            bput_sv_string(aTHX_ out, name ? *name : &PL_sv_no);
        }
        return 1;
    }
    if (!enc->direct) return 0;
    names = hv_fetch(enc->direct, class,
        HvNAMEUTF8(stash) ? -(I32)HvNAMELEN(stash) : (I32)HvNAMELEN(stash), 0);
    if (!names || !SvROK(*names) || SvTYPE(SvRV(*names)) != SVt_PVAV) return 0;
    if (enc->state && HvUSEDKEYS(enc->state)) {
        length = snprintf(address, sizeof address, "%" UVuf, PTR2UV(target));
        if (hv_exists(enc->state, address, length)) return 0;
    }
    list = (AV *)SvRV(*names);
    last = av_len(list);
    bput_tag(aTHX_ out, B_MODEL);
    bput_bytes(aTHX_ out, class, HvNAMELEN(stash));
    count_at = SvCUR(out);
    bput_len(aTHX_ out, 0);
    for (i = 0; i <= last; i++) {
        SV **name = av_fetch(list, i, 0);
        HE *field;
        if (!name) continue;
        if (!SvIsCOW_shared_hash(*name)) {
            /* a shared hash key carries its hash: every later lookup skips hashing the name */
            STRLEN len;
            const char *text = SvPV_const(*name, len);
            SV *shared = newSVpvn_share(text, SvUTF8(*name) ? -(I32)len : (I32)len, 0);
            av_store(list, i, shared);
            name = &AvARRAY(list)[i];
        }
        field = hv_fetch_ent((HV *)target, *name, 0, 0);
        if (!field) continue;
        bput_sv_string(aTHX_ out, *name);
        bemit(aTHX_ out, HeVAL(field), enc, depth + 1);
        count++;
    }
    {
        unsigned char *raw = (unsigned char *)SvPVX(out) + count_at;
        raw[0] = count & 0xff; raw[1] = (count >> 8) & 0xff;
        raw[2] = (count >> 16) & 0xff; raw[3] = (count >> 24) & 0xff;
    }
    return 1;
}

/* Hash entries in the order of their key bytes (sv_cmp's order for keys that are not UTF-8). */
static int compare_entries(const void *a, const void *b)
{
    const HE *x = *(HE *const *)a, *y = *(HE *const *)b;
    I32 lx = HeKLEN(x), ly = HeKLEN(y);
    int c = memcmp(HeKEY(x), HeKEY(y), lx < ly ? lx : ly);
    return c ? c : (lx < ly ? -1 : lx > ly);
}

/* A hash key as UTF-8 bytes: keys that are not UTF-8 are Latin-1 characters. */
static void bput_key(pTHX_ SV *out, const char *key, I32 len)
{
    I32 i;
    for (i = 0; i < len; i++) {
        if ((unsigned char)key[i] & 0x80) {
            STRLEN utf8_len;
            SV *copy = sv_2mortal(newSVpvn(key, len));
            const char *utf8 = SvPVutf8(copy, utf8_len);
            bput_bytes(aTHX_ out, utf8, utf8_len);
            return;
        }
    }
    bput_bytes(aTHX_ out, key, len);
}

static void bemit_hash(pTHX_ SV *out, HV *hash, encoder *enc, int depth)
{
    I32 count = hv_iterinit(hash);
    HE **entries;
    HE *entry;
    I32 i = 0;
    int plain = !SvRMAGICAL((SV *)hash);
    Newx(entries, count > 0 ? count : 1, HE *);
    SAVEFREEPV(entries);
    while (plain && (entry = hv_iternext(hash)) && i < count) {
        /* UTF-8 keys sort among the others as characters: sv_cmp below knows how */
        if (HeKLEN(entry) == HEf_SVKEY || HeKUTF8(entry)) plain = 0;
        entries[i++] = entry;
    }
    if (!plain) {
        SV **keys;
        Newx(keys, count > 0 ? count : 1, SV *);
        SAVEFREEPV(keys);
        hv_iterinit(hash);
        i = 0;
        while ((entry = hv_iternext(hash)) && i < count) keys[i++] = hv_iterkeysv(entry);
        count = i;
        qsort(keys, count, sizeof(SV *), compare_keys);
        bput_tag(aTHX_ out, B_DICT);
        bput_len(aTHX_ out, count);
        for (i = 0; i < count; i++) {
            HE *found = hv_fetch_ent(hash, keys[i], 0, 0);
            bput_sv_string(aTHX_ out, keys[i]);
            bemit(aTHX_ out, found ? HeVAL(found) : &PL_sv_undef, enc, depth + 1);
        }
        return;
    }
    count = i;
    qsort(entries, count, sizeof(HE *), compare_entries);
    bput_tag(aTHX_ out, B_DICT);
    bput_len(aTHX_ out, count);
    for (i = 0; i < count; i++) {
        bput_key(aTHX_ out, HeKEY(entries[i]), HeKLEN(entries[i]));
        bemit(aTHX_ out, HeVAL(entries[i]), enc, depth + 1);
    }
}

static void bemit(pTHX_ SV *out, SV *value, encoder *enc, int depth)
{
    SvGETMAGIC(value);
    if (depth > MAX_DEPTH) { bput_fallback(aTHX_ out, value, enc); return; }
    if (SvROK(value)) {
        SV *target = SvRV(value);
        if (SvOBJECT(target)) {
            if (SvTYPE(target) == SVt_PVHV
                && (bemit_model(aTHX_ out, target, enc, depth) || bemit_enum(aTHX_ out, target, enc, depth)))
                return;
            bput_fallback(aTHX_ out, value, enc);
            return;
        }
        if (SvRMAGICAL(target)) { bput_fallback(aTHX_ out, value, enc); return; }
        if (SvTYPE(target) == SVt_PVAV) {
            AV *array = (AV *)target;
            SSize_t last = av_len(array), i;
            bput_tag(aTHX_ out, B_LIST);
            bput_len(aTHX_ out, (STRLEN)(last + 1));
            for (i = 0; i <= last; i++) {
                SV **item = av_fetch(array, i, 0);
                bemit(aTHX_ out, item ? *item : &PL_sv_undef, enc, depth + 1);
            }
            return;
        }
        if (SvTYPE(target) == SVt_PVHV) {
            ENTER;
            bemit_hash(aTHX_ out, (HV *)target, enc, depth);
            LEAVE;
            return;
        }
        bput_fallback(aTHX_ out, value, enc);
        return;
    }
    if (!SvOK(value)) { bput_tag(aTHX_ out, B_NONE); return; }
#ifdef SvIsBOOL
    if (SvIsBOOL(value)) { bput_tag(aTHX_ out, SvTRUE_nomg(value) ? B_TRUE : B_FALSE); return; }
#endif
    if ((SvIOK(value) || SvNOK(value)) && !SvPOK(value)) {
        if (SvIOK(value)) {
            if (SvIsUV(value) && SvUV_nomg(value) > (UV)IV_MAX) {
                /* beyond i64: the digits as a JSON number, which the core reads as a big int */
                SV *digits = sv_2mortal(newSVpvf("%" UVuf, SvUV_nomg(value)));
                bput_tag(aTHX_ out, B_JSON);
                bput_bytes(aTHX_ out, SvPVX(digits), SvCUR(digits));
            } else {
                bput_tag(aTHX_ out, B_INT);
                bput_u64(aTHX_ out, (U64)SvIV_nomg(value));
            }
        } else {
            NV nv = SvNV_nomg(value);
            double d = (double)nv;
            U64 bits;
            memcpy(&bits, &d, 8);
            bput_tag(aTHX_ out, B_FLOAT);
            bput_u64(aTHX_ out, bits);
        }
        return;
    }
    bput_tag(aTHX_ out, B_STR);
    bput_sv_string(aTHX_ out, value);
}

/* Reading a result buffer back into Perl values. */
typedef struct {
    const unsigned char *at;
    const unsigned char *end;
    SV *json_decoder;
    HV *bless; /* %Perldantic::Wire::BLESS: class => number of fields */
    HV *lazy;  /* %Perldantic::Wire::LAZY: class => whether its objects may be lazy */
    UV pending; /* values read so far that Perldantic::Model::_inflate must still turn into
                   objects: a model holding one is left to Perl too, which does not look
                   inside objects */
} breader;

static void breader_init(pTHX_ breader *r, const unsigned char *at, const unsigned char *end,
                         SV *json_decoder)
{
    r->at = at;
    r->end = end;
    r->json_decoder = json_decoder;
    dMY_CXT;
    r->bless = MY_CXT.bless;
    r->lazy = MY_CXT.lazy;
    r->pending = 0;
}

/* Whether a validated model can be blessed as it is: its class is registered, it set every
   field (without extra values, the names set are field names) and carries no input-object
   token (names starting with NUL). */
static int bset_all(pTHX_ breader *r, SV *class, SV *fields_set, SV *extra);

static int bblessable(pTHX_ breader *r, SV *class, SV *fields, SV *fields_set, SV *extra)
{
    if (!SvROK(fields) || SvOBJECT(SvRV(fields)) || SvTYPE(SvRV(fields)) != SVt_PVHV) return 0;
    return bset_all(aTHX_ r, class, fields_set, extra);
}

/* The conditions of bblessable on all but the fields hash. */
static int bset_all(pTHX_ breader *r, SV *class, SV *fields_set, SV *extra)
{
    HE *count;
    AV *names;
    SSize_t last, i;
    if (!r->bless || SvOK(extra)) return 0;
    if (!SvROK(fields_set) || SvTYPE(SvRV(fields_set)) != SVt_PVAV) return 0;
    count = hv_fetch_ent(r->bless, class, 0, 0);
    if (!count) return 0;
    names = (AV *)SvRV(fields_set);
    last = av_len(names);
    if (last + 1 < SvIV(HeVAL(count))) return 0;
    for (i = 0; i <= last; i++) {
        SV **name = av_fetch(names, i, 0);
        STRLEN len;
        const char *text;
        if (!name) return 0;
        text = SvPV_const(*name, len);
        if (len > 0 && text[0] == '\0') return 0;
    }
    return 1;
}

static void bneed(pTHX_ breader *r, STRLEN count)
{
    if ((STRLEN)(r->end - r->at) < count) croak("Perldantic::XS::decode_result: truncated buffer");
}

static U32 bget_len(pTHX_ breader *r)
{
    U32 n;
    bneed(aTHX_ r, 4);
    n = (U32)r->at[0] | ((U32)r->at[1] << 8) | ((U32)r->at[2] << 16) | ((U32)r->at[3] << 24);
    r->at += 4;
    return n;
}

static U64 bget_u64(pTHX_ breader *r)
{
    U64 bits = 0;
    int i;
    bneed(aTHX_ r, 8);
    for (i = 0; i < 8; i++) bits |= (U64)r->at[i] << (8 * i);
    r->at += 8;
    return bits;
}

/* A UTF-8 string, flagged as characters when it has any beyond ASCII (as JSON decoders do). */
static SV *bget_string(pTHX_ breader *r)
{
    U32 len = bget_len(aTHX_ r);
    SV *sv;
    U32 i;
    int ascii = 1;
    bneed(aTHX_ r, len);
    for (i = 0; i < len; i++) if (r->at[i] & 0x80) { ascii = 0; break; }
    sv = newSVpvn((const char *)r->at, len);
    if (!ascii) SvUTF8_on(sv);
    r->at += len;
    return sv;
}

static SV *bread(pTHX_ breader *r, int depth);

/* The contents of a lazy model from the core, in a Perl copy (so a dying reader leaks nothing),
   after the result kind byte. */
static SV *lazy_contents(pTHX_ U64 handle)
{
    size_t len = 0;
    unsigned char *buffer;
    SV *copy;
    if (!core_lazy_contents || !core_buffer_free) croak("Perldantic::XS: the core is not bound");
    buffer = core_lazy_contents(handle, &len);
    if (!buffer || len == 0) croak("Perldantic::XS: empty result from the core");
    copy = sv_2mortal(newSVpvn((const char *)buffer, len));
    core_buffer_free(buffer, len);
    if (SvPVX(copy)[0] != 'B') croak("Perldantic::XS: %s", SvPVX(copy) + 1);
    return copy;
}

/* Whether objects of a class may be lazy, asking Perldantic::Model::_lazy_plan for classes it
   has not planned yet. */
static int lazy_class(pTHX_ breader *r, SV *class)
{
    HE *entry;
    if (!r->lazy) return 0;
    entry = hv_fetch_ent(r->lazy, class, 0, 0);
    if (!entry) {
        CV *plan = get_cv("Perldantic::Model::_lazy_plan", 0);
        dSP;
        if (!plan) return 0;
        ENTER;
        SAVETMPS;
        PUSHMARK(SP);
        XPUSHs(class);
        PUTBACK;
        call_sv((SV *)plan, G_DISCARD);
        FREETMPS;
        LEAVE;
        entry = hv_fetch_ent(r->lazy, class, 0, 0);
    }
    return entry && SvTRUE(HeVAL(entry));
}

/* A lazy node: a lazy object when its class allows, else the model read in full now. */
static SV *bread_lazy(pTHX_ breader *r, int depth)
{
    SV *class = sv_2mortal(bget_string(aTHX_ r));
    U64 handle = bget_u64(aTHX_ r);
    U32 names = bget_len(aTHX_ r), i;
    HV *stash;
    /* names only come from hosts sending models back */
    for (i = 0; i < names; i++) {
        U32 len = bget_len(aTHX_ r);
        bneed(aTHX_ r, len);
        r->at += len;
    }
    if (lazy_class(aTHX_ r, class) && (stash = gv_stashsv(class, 0)) != NULL) {
        HV *object = newHV();
        SV *ref = newRV_noinc((SV *)object);
        MAGIC *mg = sv_magicext((SV *)object, NULL, PERL_MAGIC_ext, &lazy_vtbl, NULL, 0);
        mg->mg_ptr = INT2PTR(char *, (UV)handle);
#ifdef USE_ITHREADS
        mg->mg_flags |= MGf_DUP;
#endif
        return sv_bless(ref, stash);
    } else {
        SV *copy, *value;
        breader inner;
        /* released once copied: a dying reader must not leave the handle behind */
        copy = lazy_contents(aTHX_ handle);
        if (core_lazy_release) core_lazy_release(handle);
        inner = *r;
        inner.at = (const unsigned char *)SvPVX(copy) + 1;
        inner.end = (const unsigned char *)SvPVX(copy) + SvCUR(copy);
        value = bread(aTHX_ &inner, depth + 1);
        r->pending = inner.pending;
        if (inner.at != inner.end) {
            SvREFCNT_dec(value);
            croak("Perldantic::XS: trailing bytes in a lazy model");
        }
        return value;
    }
}

static void bread_entries_into(pTHX_ breader *r, int depth, HV *hash)
{
    U32 count = bget_len(aTHX_ r), i;
    for (i = 0; i < count; i++) {
        U32 len = bget_len(aTHX_ r), j;
        const char *key;
        int ascii = 1;
        SV *value;
        bneed(aTHX_ r, len);
        key = (const char *)r->at;
        for (j = 0; j < len; j++) if (r->at[j] & 0x80) { ascii = 0; break; }
        r->at += len;
        value = bread(aTHX_ r, depth + 1);
        /* a negative length marks a UTF-8 key; Perl stores it downgraded when it can */
        if (!hv_store(hash, key, ascii ? (I32)len : -(I32)len, value, 0)) SvREFCNT_dec(value);
    }
}

static HV *bread_entries(pTHX_ breader *r, int depth)
{
    HV *hash = newHV();
    bread_entries_into(aTHX_ r, depth, hash);
    return hash;
}

static SV *bread(pTHX_ breader *r, int depth)
{
    unsigned char tag;
    if (depth > MAX_DEPTH + 1) croak("Perldantic::XS::decode_result: nested too deeply");
    bneed(aTHX_ r, 1);
    tag = *r->at++;
    switch (tag) {
    case B_NONE: return newSV(0);
    case B_TRUE: return newSVsv(&PL_sv_yes);
    case B_FALSE: return newSVsv(&PL_sv_no);
    case B_INT: return newSViv((IV)bget_u64(aTHX_ r));
    case B_FLOAT: {
        U64 bits = bget_u64(aTHX_ r);
        double d;
        memcpy(&d, &bits, 8);
        return newSVnv((NV)d);
    }
    case B_STR: return bget_string(aTHX_ r);
    case B_BYTES: {
        U32 len = bget_len(aTHX_ r);
        SV *sv;
        bneed(aTHX_ r, len);
        sv = newSVpvn((const char *)r->at, len);
        r->at += len;
        return sv;
    }
    case B_LIST: {
        U32 count = bget_len(aTHX_ r), i;
        AV *array = newAV();
        SV *ref = newRV_noinc((SV *)array);
        if (count) av_extend(array, count > 4096 ? 4096 : count - 1);
        for (i = 0; i < count; i++) av_push(array, bread(aTHX_ r, depth + 1));
        return ref;
    }
    case B_DICT: return newRV_noinc((SV *)bread_entries(aTHX_ r, depth));
    case B_JSON: {
        U32 len = bget_len(aTHX_ r);
        SV *json, *value;
        dSP;
        int count;
        bneed(aTHX_ r, len);
        json = sv_2mortal(newSVpvn((const char *)r->at, len));
        r->at += len;
        r->pending++;
        ENTER;
        SAVETMPS;
        PUSHMARK(SP);
        XPUSHs(json);
        PUTBACK;
        count = call_sv(r->json_decoder, G_SCALAR);
        SPAGAIN;
        if (count != 1) croak("Perldantic::XS::decode_result: the JSON decoder returned no value");
        value = newSVsv(POPs);
        PUTBACK;
        FREETMPS;
        LEAVE;
        return value;
    }
    case B_MODEL: {
        /* a model that set exactly the fields it holds, with no extra values */
        SV *class = bget_string(aTHX_ r);
        UV pending = r->pending;
        HV *fields = bread_entries(aTHX_ r, depth);
        SV *fields_ref = newRV_noinc((SV *)fields);
        HE *count = r->bless ? hv_fetch_ent(r->bless, class, 0, 0) : NULL;
        HV *stash;
        if (count && r->pending == pending && HvUSEDKEYS(fields) >= SvIV(HeVAL(count))
            && (stash = gv_stashsv(class, 0)) != NULL) {
            SvREFCNT_dec(class);
            return sv_bless(fields_ref, stash);
        } else {
            HV *model = newHV();
            SV *ref = newRV_noinc((SV *)model);
            AV *names = newAV();
            HE *entry;
            hv_iterinit(fields);
            while ((entry = hv_iternext(fields))) av_push(names, newSVsv(hv_iterkeysv(entry)));
            sortsv(AvARRAY(names), av_len(names) + 1, Perl_sv_cmp);
            (void)hv_stores(model, "class", class);
            (void)hv_stores(model, "fields", fields_ref);
            (void)hv_stores(model, "fields_set", newRV_noinc((SV *)names));
            (void)hv_stores(model, "extra", newSV(0));
            r->pending++;
            return sv_bless(ref, gv_stashpvs("Perldantic::Wire::Model", GV_ADD));
        }
    }
    case B_MODEL_FULL: {
        SV *class = bget_string(aTHX_ r);
        UV pending = r->pending;
        SV *fields = bread(aTHX_ r, depth + 1);
        SV *fields_set = bread(aTHX_ r, depth + 1);
        SV *extra = bread(aTHX_ r, depth + 1);
        HV *model, *stash;
        SV *ref;
        if (r->pending == pending && bblessable(aTHX_ r, class, fields, fields_set, extra)
            && (stash = gv_stashsv(class, 0)) != NULL) {
            /* the fields hash is the object, as Perldantic::Model::_inflate makes it */
            SvREFCNT_dec(class);
            SvREFCNT_dec(fields_set);
            SvREFCNT_dec(extra);
            return sv_bless(fields, stash);
        }
        model = newHV();
        ref = newRV_noinc((SV *)model);
        (void)hv_stores(model, "class", class);
        (void)hv_stores(model, "fields", fields);
        (void)hv_stores(model, "fields_set", fields_set);
        (void)hv_stores(model, "extra", extra);
        r->pending++;
        return sv_bless(ref, gv_stashpvs("Perldantic::Wire::Model", GV_ADD));
    }
    case B_LAZY: return bread_lazy(aTHX_ r, depth);
    case B_ENUM: {
        /* the member itself for a Perl enum class, else a Perldantic::Wire::Enum */
        SV *class = sv_2mortal(bget_string(aTHX_ r));
        SV *name = sv_2mortal(bget_string(aTHX_ r));
        SV *mixin = sv_2mortal(bget_string(aTHX_ r));
        int str_is_value;
        SV *value;
        HV *entry, *member;
        bneed(aTHX_ r, 1);
        str_is_value = *r->at++ != 0;
        value = bread(aTHX_ r, depth + 1);
        if ((entry = enum_class(aTHX_ class)) != NULL) {
            SV **by_name = hv_fetchs(entry, "by_name", 0);
            HE *found = by_name && SvROK(*by_name) && SvTYPE(SvRV(*by_name)) == SVt_PVHV
                ? hv_fetch_ent((HV *)SvRV(*by_name), name, 0, 0) : NULL;
            if (found) {
                SvREFCNT_dec(value);
                return newSVsv(HeVAL(found));
            }
        }
        member = newHV();
        (void)hv_stores(member, "class", newSVsv(class));
        (void)hv_stores(member, "name", newSVsv(name));
        (void)hv_stores(member, "value", value);
        (void)hv_stores(member, "mixin", SvCUR(mixin) ? newSVsv(mixin) : newSV(0));
        (void)hv_stores(member, "str_is_value", newSVsv(str_is_value ? &PL_sv_yes : &PL_sv_no));
        return sv_bless(newRV_noinc((SV *)member), gv_stashpvs("Perldantic::Wire::Enum", GV_ADD));
    }
    default:
        croak("Perldantic::XS::decode_result: unknown tag %d", (int)tag);
    }
    return NULL;
}

/* The encoder state for one call, read from the Perl side's globals. */
static void encoder_init(pTHX_ encoder *enc, SV *fallback)
{
    SV *tracking = get_sv("Perldantic::Model::TRACK_OBJECTS", 0);
    enc->fallback = fallback;
    enc->direct = get_hv("Perldantic::Wire::DIRECT", 0);
    enc->lazy_dump = get_hv("Perldantic::Wire::LAZY_DUMP", 0);
    enc->state = get_hv("Perldantic::Model::STATE", 0);
    enc->tracking = tracking && SvTRUE(tracking);
    enc->guarded = 0;
    enc->error = NULL;
}

/*
 * Host data read in place by the core (ffi/src/host_input.rs): the core walks Perl arrays and
 * hashes through these functions instead of receiving a copy, and asks for anything else
 * (scalars, objects, tied containers) in the binary wire format. Nodes are SV pointers, valid
 * for the whole call; strings handed over live in mortal SVs, freed when the call returns.
 */
/* A plain scalar described without allocating (ffi/src/host_input.rs, PdScalar). */
typedef struct {
    int tag;          /* 0 undef, 1 true, 2 false, 3 integer, 4 float, 5 UTF-8 string */
    long long integer;
    double number;
    const unsigned char *ptr;
    size_t len;
} pd_scalar;

typedef struct {
    void *ctx;
    int (*kind)(void *ctx, void *node);
    size_t (*array_len)(void *ctx, void *node);
    void *(*array_item)(void *ctx, void *node, size_t index);
    void *(*hash_get)(void *ctx, void *node, const unsigned char *key, size_t key_len);
    size_t (*hash_len)(void *ctx, void *node);
    size_t (*hash_entries)(void *ctx, void *node, const unsigned char **keys, size_t *key_lens,
                           void **values, size_t capacity);
    int (*scalar)(void *ctx, void *node, pd_scalar *out);
    int (*to_binary)(void *ctx, void *node, const unsigned char **bytes, size_t *len);
} pd_host;

typedef unsigned char *(*pd_validate_host_fn)(void *validator, const pd_host *host, void *root,
                                              const char *options, size_t *len);

/* The core's exports, bound once by Perldantic::FFI (process-wide function addresses). */
static pd_validate_host_fn core_validate_host = NULL;
static pd_validate_host_fn core_check_host = NULL;
static pd_validate_host_fn core_validate_host_lazy = NULL;

/* The container behind a node that the core may read in place: a reference to a plain (not
   blessed, not tied) array or hash. */
static SV *host_container(SV *node)
{
    SV *target;
    if (!SvROK(node)) return NULL;
    target = SvRV(node);
    if (SvOBJECT(target) || SvRMAGICAL(target)) return NULL;
    if (SvTYPE(target) != SVt_PVAV && SvTYPE(target) != SVt_PVHV) return NULL;
    return target;
}

static int host_kind(void *ctx, void *node)
{
    SV *target = host_container((SV *)node);
    PERL_UNUSED_ARG(ctx);
    if (!target) return 0;
    return SvTYPE(target) == SVt_PVAV ? 1 : 2;
}

static size_t host_array_len(void *ctx, void *node)
{
    SV *target = host_container((SV *)node);
    dTHX;
    PERL_UNUSED_ARG(ctx);
    return target && SvTYPE(target) == SVt_PVAV ? (size_t)(av_len((AV *)target) + 1) : 0;
}

static void *host_array_item(void *ctx, void *node, size_t index)
{
    SV *target = host_container((SV *)node);
    SV **item;
    dTHX;
    PERL_UNUSED_ARG(ctx);
    if (!target || SvTYPE(target) != SVt_PVAV) return &PL_sv_undef;
    item = av_fetch((AV *)target, (SSize_t)index, 0);
    return item ? *item : &PL_sv_undef;
}

static void *host_hash_get(void *ctx, void *node, const unsigned char *key, size_t key_len)
{
    SV *target = host_container((SV *)node);
    SV **found;
    size_t i;
    int ascii = 1;
    dTHX;
    PERL_UNUSED_ARG(ctx);
    if (!target || SvTYPE(target) != SVt_PVHV) return NULL;
    for (i = 0; i < key_len; i++) if (key[i] & 0x80) { ascii = 0; break; }
    /* a negative length marks UTF-8 key bytes; Perl finds the key however it is stored */
    found = hv_fetch((HV *)target, (const char *)key, ascii ? (I32)key_len : -(I32)key_len, 0);
    return found ? *found : NULL;
}

static size_t host_hash_len(void *ctx, void *node)
{
    SV *target = host_container((SV *)node);
    dTHX;
    PERL_UNUSED_ARG(ctx);
    return target && SvTYPE(target) == SVt_PVHV ? (size_t)HvUSEDKEYS((HV *)target) : 0;
}

/* The entries of a hash in the order the binary encoder writes them (sorted keys), keys as
   UTF-8 bytes. */
static size_t host_hash_entries(void *ctx, void *node, const unsigned char **keys,
                                size_t *key_lens, void **values, size_t capacity)
{
    SV *target = host_container((SV *)node);
    HV *hash;
    HE **entries;
    HE *entry;
    size_t count = 0, i;
    int plain = 1;
    dTHX;
    PERL_UNUSED_ARG(ctx);
    if (!target || SvTYPE(target) != SVt_PVHV || capacity == 0) return 0;
    hash = (HV *)target;
    Newx(entries, capacity, HE *);
    SAVEFREEPV(entries);
    hv_iterinit(hash);
    while ((entry = hv_iternext(hash)) && count < capacity) {
        if (HeKLEN(entry) == HEf_SVKEY || HeKUTF8(entry)) plain = 0;
        entries[count++] = entry;
    }
    if (plain) {
        qsort(entries, count, sizeof(HE *), compare_entries);
        for (i = 0; i < count; i++) {
            const char *key = HeKEY(entries[i]);
            I32 len = HeKLEN(entries[i]), j;
            for (j = 0; j < len; j++) if ((unsigned char)key[j] & 0x80) break;
            if (j < len) {
                /* Latin-1 bytes: the key as UTF-8, in a mortal copy */
                STRLEN utf8_len;
                SV *copy = sv_2mortal(newSVpvn(key, len));
                keys[i] = (const unsigned char *)SvPVutf8(copy, utf8_len);
                key_lens[i] = utf8_len;
            } else {
                keys[i] = (const unsigned char *)key;
                key_lens[i] = (size_t)len;
            }
            values[i] = HeVAL(entries[i]);
        }
        return count;
    } else {
        /* UTF-8 keys: sort key SVs as the binary encoder does */
        SV **key_svs;
        Newx(key_svs, count, SV *);
        SAVEFREEPV(key_svs);
        for (i = 0; i < count; i++) key_svs[i] = hv_iterkeysv(entries[i]);
        qsort(key_svs, count, sizeof(SV *), compare_keys);
        for (i = 0; i < count; i++) {
            HE *found = hv_fetch_ent(hash, key_svs[i], 0, 0);
            STRLEN len;
            SV *copy = sv_2mortal(newSVsv(key_svs[i]));
            keys[i] = (const unsigned char *)SvPVutf8(copy, len);
            key_lens[i] = len;
            values[i] = found ? HeVAL(found) : &PL_sv_undef;
        }
        return count;
    }
}

/* A plain scalar as the binary encoder would write it; 0 for anything else (references,
   magic, integers beyond 64 bits, Latin-1 strings), which then goes through host_to_binary. */
static int host_scalar(void *ctx, void *node, pd_scalar *out)
{
    SV *sv = (SV *)node;
    PERL_UNUSED_ARG(ctx);
    if (SvROK(sv) || SvGMAGICAL(sv)) return 0;
    if (!SvOK(sv)) { out->tag = 0; return 1; }
#ifdef SvIsBOOL
    if (SvIsBOOL(sv)) { out->tag = SvIV_nomg(sv) ? 1 : 2; return 1; }
#endif
    if ((SvIOK(sv) || SvNOK(sv)) && !SvPOK(sv)) {
        if (SvIOK(sv)) {
            if (SvIsUV(sv) && SvUVX(sv) > (UV)IV_MAX) return 0;
            out->tag = 3;
            out->integer = (long long)SvIVX(sv);
        } else {
            out->tag = 4;
            out->number = (double)SvNVX(sv);
        }
        return 1;
    }
    if (SvPOK(sv)) {
        const unsigned char *text = (const unsigned char *)SvPVX(sv);
        STRLEN len = SvCUR(sv), i;
        if (!SvUTF8(sv)) for (i = 0; i < len; i++) if (text[i] & 0x80) return 0;
        out->tag = 5;
        out->ptr = text;
        out->len = len;
        return 1;
    }
    return 0;
}

static int host_to_binary(void *ctx, void *node, const unsigned char **bytes, size_t *len)
{
    encoder *enc = (encoder *)ctx;
    SV *out;
    dTHX;
    out = sv_2mortal(newSVpvn("", 0));
    bemit(aTHX_ out, (SV *)node, enc, 0);
    if (enc->error) return 0;
    *bytes = (const unsigned char *)SvPVX(out);
    *len = SvCUR(out);
    return 1;
}

/* A result buffer of the core as ('ok', $value, $warning) or ('envelope', $json), pushed on
   the stack; the buffer is read from a Perl copy, so a dying decoder leaks nothing. */
static int push_result(pTHX_ SV **sp_in, SV *copy, SV *json_decoder)
{
    SV **sp = sp_in;
    breader r;
    const unsigned char *bytes = (const unsigned char *)SvPVX(copy);
    STRLEN len = SvCUR(copy);
    if (len == 0) croak("Perldantic::XS: empty result from the core");
    breader_init(aTHX_ &r, bytes + 1, bytes + len, json_decoder);
    if (bytes[0] == 'J') {
        EXTEND(SP, 2);
        mPUSHs(newSVpvs("envelope"));
        mPUSHs(newSVpvn((const char *)r.at, (STRLEN)(r.end - r.at)));
        PUTBACK;
        return 2;
    }
    if (bytes[0] == 'B' || bytes[0] == 'W') {
        SV *warning = NULL, *value;
        if (bytes[0] == 'W') warning = sv_2mortal(bget_string(aTHX_ &r));
        value = sv_2mortal(bread(aTHX_ &r, 0));
        if (r.at != r.end) croak("Perldantic::XS: trailing bytes in a result of the core");
        EXTEND(SP, 3);
        PUSHs(sv_2mortal(newSVpvs("ok")));
        PUSHs(value);
        PUSHs(warning ? warning : sv_newmortal());
        PUTBACK;
        return 3;
    }
    croak("Perldantic::XS: unknown result kind %d", (int)bytes[0]);
    return 0;
}

MODULE = Perldantic    PACKAGE = Perldantic::XS

PROTOTYPES: DISABLE

BOOT:
{
    MY_CXT_INIT;
    cxt_fetch(aTHX_ &MY_CXT);
}

void
CLONE(...)
    CODE:
    {
        MY_CXT_CLONE;
        cxt_fetch(aTHX_ &MY_CXT);
        PERL_UNUSED_VAR(items);
    }

SV *
encode(value, fallback)
        SV *value
        SV *fallback
    PREINIT:
        encoder enc;
    CODE:
        encoder_init(aTHX_ &enc, fallback);
        RETVAL = newSVpvn("", 0);
        emit(aTHX_ RETVAL, value, &enc, 0);
    OUTPUT:
        RETVAL

SV *
encode_pairs(pairs, fallback)
        SV *pairs
        SV *fallback
    PREINIT:
        encoder enc;
    CODE:
        if (!SvROK(pairs) || SvTYPE(SvRV(pairs)) != SVt_PVAV)
            croak("Perldantic::XS::encode_pairs takes an array reference");
        encoder_init(aTHX_ &enc, fallback);
        RETVAL = newSVpvn("", 0);
        emit_pairs(aTHX_ RETVAL, (AV *)SvRV(pairs), &enc);
    OUTPUT:
        RETVAL

SV *
encode_binary(value, fallback)
        SV *value
        SV *fallback
    PREINIT:
        encoder enc;
    CODE:
        encoder_init(aTHX_ &enc, fallback);
        RETVAL = newSVpvn("", 0);
        bemit(aTHX_ RETVAL, value, &enc, 0);
    OUTPUT:
        RETVAL

void
decode_result(address, len, json_decoder)
        UV address
        UV len
        SV *json_decoder
    PREINIT:
        breader r;
        const unsigned char *bytes;
    PPCODE:
        /* A result buffer of a binary export: ('ok', $value, $warning) or ('envelope', $json). */
        if (!address || !len) croak("Perldantic::XS::decode_result: empty buffer");
        bytes = INT2PTR(const unsigned char *, address);
        breader_init(aTHX_ &r, bytes + 1, bytes + len, json_decoder);
        if (bytes[0] == 'J') {
            EXTEND(SP, 2);
            mPUSHs(newSVpvs("envelope"));
            mPUSHs(newSVpvn((const char *)r.at, (STRLEN)(r.end - r.at)));
        } else if (bytes[0] == 'B' || bytes[0] == 'W') {
            SV *warning = NULL, *value;
            if (bytes[0] == 'W') warning = bget_string(aTHX_ &r);
            value = bread(aTHX_ &r, 0);
            if (r.at != r.end) {
                SvREFCNT_dec(value);
                if (warning) SvREFCNT_dec(warning);
                croak("Perldantic::XS::decode_result: trailing bytes");
            }
            EXTEND(SP, 3);
            mPUSHs(newSVpvs("ok"));
            mPUSHs(value);
            mPUSHs(warning ? warning : newSV(0));
        } else {
            croak("Perldantic::XS::decode_result: unknown result kind %d", (int)bytes[0]);
        }

void
bind_core(validate_host, check_host, validate_host_lazy, buffer_free, lazy_contents, lazy_clone, lazy_release)
        UV validate_host
        UV check_host
        UV validate_host_lazy
        UV buffer_free
        UV lazy_contents
        UV lazy_clone
        UV lazy_release
    CODE:
        /* the core's exports, found by Perldantic::FFI */
        core_validate_host = INT2PTR(pd_validate_host_fn, validate_host);
        core_check_host = INT2PTR(pd_validate_host_fn, check_host);
        core_validate_host_lazy = INT2PTR(pd_validate_host_fn, validate_host_lazy);
        core_buffer_free = INT2PTR(pd_buffer_free_fn, buffer_free);
        core_lazy_contents = INT2PTR(pd_lazy_contents_fn, lazy_contents);
        core_lazy_clone = INT2PTR(pd_lazy_clone_fn, lazy_clone);
        core_lazy_release = INT2PTR(pd_lazy_release_fn, lazy_release);

void
lazy_expand(object, json_decoder)
        SV *object
        SV *json_decoder
    PREINIT:
        SV *target, *copy, *class, *fields, *fields_set, *extra;
        MAGIC *mg;
        breader r;
        U64 handle;
    PPCODE:
        /* Read a lazy object in, its handle released: models in its fields are new lazy
           objects. Returns ($fields, $fields_set, $extra) for Perl to finish the object with,
           or an empty list when there is nothing to finish (or the value is not lazy). */
        if (!SvROK(object)) XSRETURN_EMPTY;
        target = SvRV(object);
        if (SvTYPE(target) != SVt_PVHV || !(mg = lazy_magic(aTHX_ target))) XSRETURN_EMPTY;
        handle = (U64)PTR2UV(mg->mg_ptr);
        copy = lazy_contents(aTHX_ handle);
        breader_init(aTHX_ &r, (const unsigned char *)SvPVX(copy) + 1,
            (const unsigned char *)SvPVX(copy) + SvCUR(copy), json_decoder);
        bneed(aTHX_ &r, 1);
        if (*r.at++ != B_MODEL_FULL) croak("Perldantic::XS::lazy_expand: not a model");
        class = sv_2mortal(bget_string(aTHX_ &r));
        bneed(aTHX_ &r, 1);
        if (*r.at == B_DICT && !HvUSEDKEYS((HV *)target)) {
            /* the fields go straight into the object */
            r.at++;
            bread_entries_into(aTHX_ &r, 1, (HV *)target);
            fields = sv_2mortal(newRV_inc(target));
        } else {
            fields = sv_2mortal(bread(aTHX_ &r, 1));
        }
        fields_set = sv_2mortal(bread(aTHX_ &r, 1));
        extra = sv_2mortal(bread(aTHX_ &r, 1));
        if (r.at != r.end) croak("Perldantic::XS::lazy_expand: trailing bytes");
        sv_unmagicext(target, PERL_MAGIC_ext, &lazy_vtbl);
        if (SvRV(fields) == target && r.pending == 0 && bset_all(aTHX_ &r, class, fields_set, extra)) {
            /* every field given, nothing for Perl to build */
            XSRETURN_EMPTY;
        }
        EXTEND(SP, 3);
        PUSHs(fields);
        PUSHs(fields_set);
        PUSHs(extra);

void
validate_host(validator, input, options, fallback, json_decoder, mode)
        UV validator
        SV *input
        SV *options
        SV *fallback
        SV *json_decoder
        int mode
    PREINIT:
        encoder enc;
        pd_host host;
        unsigned char *buffer;
        size_t len = 0;
        SV *copy;
        int count;
    PPCODE:
        /* Validate Perl data read in place: ('ok', $value, $warning) or ('envelope', $json).
           Mode 0 builds the value, 1 only checks, 2 returns lazy objects. */
        if (!core_validate_host || !core_check_host || !core_validate_host_lazy || !core_buffer_free)
            croak("Perldantic::XS::validate_host: the core is not bound");
        encoder_init(aTHX_ &enc, fallback);
        enc.guarded = 1;
        host.ctx = &enc;
        host.kind = host_kind;
        host.array_len = host_array_len;
        host.array_item = host_array_item;
        host.hash_get = host_hash_get;
        host.hash_len = host_hash_len;
        host.hash_entries = host_hash_entries;
        host.scalar = host_scalar;
        host.to_binary = host_to_binary;
        buffer = (mode == 1 ? core_check_host : mode == 2 ? core_validate_host_lazy : core_validate_host)(
            INT2PTR(void *, validator), &host, input, SvOK(options) ? SvPV_nolen(options) : NULL, &len);
        copy = sv_2mortal(newSVpvn((const char *)buffer, len));
        core_buffer_free(buffer, len);
        if (enc.error) croak_sv(sv_2mortal(enc.error));
        PUTBACK;
        count = push_result(aTHX_ SP, copy, json_decoder);
        SPAGAIN;
        PERL_UNUSED_VAR(count);
