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

/* How deep plain data may nest before the fallback (which has no such limit) takes over. */
#define MAX_DEPTH 512

/* What one encode call needs: the fallback for values it does not write itself, and what it
   takes to write model objects natively (see emit_model). */
typedef struct {
    SV *fallback;
    HV *direct;   /* %Perldantic::Wire::DIRECT: class => [field names] */
    HV *state;    /* %Perldantic::Model::STATE, a field hash keyed by object address */
    int tracking; /* $Perldantic::Model::TRACK_OBJECTS: objects then carry tokens */
} encoder;

static void emit(pTHX_ SV *out, SV *value, encoder *enc, int depth);

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
    STRLEN len;
    const char *text;
    if (SvUTF8(value)) {
        text = SvPV_const(value, len);
    } else {
        /* bytes above 0x7f are Latin-1 characters: write them as UTF-8 */
        SV *copy = sv_2mortal(newSVsv(value));
        text = SvPVutf8(copy, len);
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
    if (enc->tracking || !enc->direct || !class) return 0;
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
    B_DICT = 7, B_JSON = 8, B_MODEL = 9, B_MODEL_FULL = 10, B_BYTES = 11
};

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
    STRLEN len;
    const char *text;
    if (SvUTF8(value)) {
        text = SvPV_const(value, len);
    } else {
        SV *copy = sv_2mortal(newSVsv(value));
        text = SvPVutf8(copy, len);
    }
    bput_bytes(aTHX_ out, text, len);
}

/* What the fallback writes for a value, as a JSON node. */
static void bput_fallback(pTHX_ SV *out, SV *value, SV *fallback)
{
    SV *json = sv_2mortal(newSVpvn("", 0));
    emit_fallback(aTHX_ json, value, fallback);
    bput_tag(aTHX_ out, B_JSON);
    bput_bytes(aTHX_ out, SvPVX(json), SvCUR(json));
}

static void bemit(pTHX_ SV *out, SV *value, encoder *enc, int depth);

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
    if (enc->tracking || !enc->direct || !class) return 0;
    names = hv_fetch(enc->direct, class,
        HvNAMEUTF8(stash) ? -(I32)HvNAMELEN(stash) : (I32)HvNAMELEN(stash), 0);
    if (!names || !SvROK(*names) || SvTYPE(SvRV(*names)) != SVt_PVAV) return 0;
    if (enc->state) {
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

static void bemit_hash(pTHX_ SV *out, HV *hash, encoder *enc, int depth)
{
    I32 count = hv_iterinit(hash);
    SV **keys;
    HE *entry;
    I32 i = 0;
    Newx(keys, count > 0 ? count : 1, SV *);
    SAVEFREEPV(keys);
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
}

static void bemit(pTHX_ SV *out, SV *value, encoder *enc, int depth)
{
    SvGETMAGIC(value);
    if (depth > MAX_DEPTH) { bput_fallback(aTHX_ out, value, enc->fallback); return; }
    if (SvROK(value)) {
        SV *target = SvRV(value);
        if (SvOBJECT(target)) {
            if (SvTYPE(target) == SVt_PVHV && bemit_model(aTHX_ out, target, enc, depth)) return;
            bput_fallback(aTHX_ out, value, enc->fallback);
            return;
        }
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
        bput_fallback(aTHX_ out, value, enc->fallback);
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
} breader;

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

static HV *bread_entries(pTHX_ breader *r, int depth)
{
    U32 count = bget_len(aTHX_ r), i;
    HV *hash = newHV();
    for (i = 0; i < count; i++) {
        SV *key = sv_2mortal(bget_string(aTHX_ r));
        SV *value = bread(aTHX_ r, depth + 1);
        if (!hv_store_ent(hash, key, value, 0)) SvREFCNT_dec(value);
    }
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
    case B_MODEL_FULL: {
        HV *model = newHV();
        SV *ref = newRV_noinc((SV *)model);
        (void)hv_stores(model, "class", bget_string(aTHX_ r));
        (void)hv_stores(model, "fields", bread(aTHX_ r, depth + 1));
        (void)hv_stores(model, "fields_set", bread(aTHX_ r, depth + 1));
        (void)hv_stores(model, "extra", bread(aTHX_ r, depth + 1));
        return sv_bless(ref, gv_stashpvs("Perldantic::Wire::Model", GV_ADD));
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
    enc->state = get_hv("Perldantic::Model::STATE", 0);
    enc->tracking = tracking && SvTRUE(tracking);
}

MODULE = Perldantic    PACKAGE = Perldantic::XS

PROTOTYPES: DISABLE

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
        r.at = bytes + 1;
        r.end = bytes + len;
        r.json_decoder = json_decoder;
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
