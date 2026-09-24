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

static void emit(pTHX_ SV *out, SV *value, SV *fallback, int depth);

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

static void emit_hash(pTHX_ SV *out, SV *ref, HV *hash, SV *fallback, int depth)
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
            emit_fallback(aTHX_ out, ref, fallback);
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
        emit(aTHX_ out, found ? HeVAL(found) : &PL_sv_undef, fallback, depth + 1);
    }
    sv_catpvn(out, "}", 1);
}

static void emit(pTHX_ SV *out, SV *value, SV *fallback, int depth)
{
    SvGETMAGIC(value);
    if (depth > MAX_DEPTH) { emit_fallback(aTHX_ out, value, fallback); return; }
    if (SvROK(value)) {
        SV *target = SvRV(value);
        if (SvOBJECT(target)) { emit_fallback(aTHX_ out, value, fallback); return; }
        if (SvTYPE(target) == SVt_PVAV) {
            AV *array = (AV *)target;
            SSize_t last = av_len(array), i;
            sv_catpvn(out, "[", 1);
            for (i = 0; i <= last; i++) {
                SV **item = av_fetch(array, i, 0);
                if (i > 0) sv_catpvn(out, ",", 1);
                emit(aTHX_ out, item ? *item : &PL_sv_undef, fallback, depth + 1);
            }
            sv_catpvn(out, "]", 1);
            return;
        }
        if (SvTYPE(target) == SVt_PVHV) {
            ENTER;
            emit_hash(aTHX_ out, value, (HV *)target, fallback, depth);
            LEAVE;
            return;
        }
        /* code references and anything else: the fallback knows (or refuses) */
        emit_fallback(aTHX_ out, value, fallback);
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
static void emit_pairs(pTHX_ SV *out, AV *pairs, SV *fallback)
{
    SSize_t last = av_len(pairs), i;
    sv_catpvn(out, "{", 1);
    for (i = 0; i + 1 <= last; i += 2) {
        SV **key = av_fetch(pairs, i, 0);
        SV **value = av_fetch(pairs, i + 1, 0);
        if (i > 0) sv_catpvn(out, ",", 1);
        emit_sv_string(aTHX_ out, key ? *key : &PL_sv_no);
        sv_catpvn(out, ":", 1);
        emit(aTHX_ out, value ? *value : &PL_sv_undef, fallback, 1);
    }
    sv_catpvn(out, "}", 1);
}

MODULE = Perldantic    PACKAGE = Perldantic::XS

PROTOTYPES: DISABLE

SV *
encode(value, fallback)
        SV *value
        SV *fallback
    CODE:
        RETVAL = newSVpvn("", 0);
        emit(aTHX_ RETVAL, value, fallback, 0);
    OUTPUT:
        RETVAL

SV *
encode_pairs(pairs, fallback)
        SV *pairs
        SV *fallback
    CODE:
        if (!SvROK(pairs) || SvTYPE(SvRV(pairs)) != SVt_PVAV)
            croak("Perldantic::XS::encode_pairs takes an array reference");
        RETVAL = newSVpvn("", 0);
        emit_pairs(aTHX_ RETVAL, (AV *)SvRV(pairs), fallback);
    OUTPUT:
        RETVAL
