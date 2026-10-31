/* site.h: URL の出どころ（scheme://host[:port]）を求める。エンジンの実装どうしで同じ規則にするため、ここに 1 つだけ書く。
 * 規則は browser/app/vault.jw の site_of と同じ: http / https だけ。host は小文字。user@ は外す。既定のポート（443 / 80）は書かない。
 * 求められなければ空（out[0] == 0）。 */
#ifndef JWE_SITE_H
#define JWE_SITE_H

#include <stddef.h>
#include <string.h>

static inline int jwe_site_lower(int c) { return c >= 'A' && c <= 'Z' ? c + 32 : c; }

static inline void jwe_site_of(const char *url, size_t n, char *out, size_t cap) {
    size_t i = 0, o = 0;
    out[0] = 0;
    while (i < n && (url[i] == ' ' || url[i] == '\t')) i++;
    const char *scheme;
    size_t skip;
    if (n - i >= 8 && jwe_site_lower(url[i]) == 'h' && jwe_site_lower(url[i + 1]) == 't' && jwe_site_lower(url[i + 2]) == 't' &&
        jwe_site_lower(url[i + 3]) == 'p' && jwe_site_lower(url[i + 4]) == 's' && url[i + 5] == ':' && url[i + 6] == '/' && url[i + 7] == '/') {
        scheme = "https";
        skip = 8;
    } else if (n - i >= 7 && jwe_site_lower(url[i]) == 'h' && jwe_site_lower(url[i + 1]) == 't' && jwe_site_lower(url[i + 2]) == 't' &&
               jwe_site_lower(url[i + 3]) == 'p' && url[i + 4] == ':' && url[i + 5] == '/' && url[i + 6] == '/') {
        scheme = "http";
        skip = 7;
    } else {
        return;
    }
    i += skip;
    size_t start = i, end = i;
    while (end < n && url[end] != '/' && url[end] != '?' && url[end] != '#') end++;
    for (size_t k = start; k < end; k++)
        if (url[k] == '@') start = k + 1;
    size_t hl = end - start;
    if (strcmp(scheme, "https") == 0 && hl >= 4 && memcmp(url + end - 4, ":443", 4) == 0) hl -= 4;
    if (strcmp(scheme, "http") == 0 && hl >= 3 && memcmp(url + end - 3, ":80", 3) == 0) hl -= 3;
    if (hl == 0 || strlen(scheme) + 3 + hl + 1 > cap) return;
    for (const char *s = scheme; *s; s++) out[o++] = *s;
    out[o++] = ':';
    out[o++] = '/';
    out[o++] = '/';
    for (size_t k = 0; k < hl; k++) out[o++] = (char)jwe_site_lower(url[start + k]);
    out[o] = 0;
}

#endif
