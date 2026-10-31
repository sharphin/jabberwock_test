/* jwe_mock.c: 試験用の偽エンジン（jw_engine.h の実装）。
 *
 * 画面は出さない。ページを読む代わりに、決まった順にイベントを出す:
 *   navigate → NAV_STARTED → URL_CHANGED → TITLE_CHANGED（"title:" + URL）→ NAV_FINISHED → HISTORY_CHANGED
 * 戻る・進むは本物と同じように、タブごとの履歴をたどる。
 *
 * 利用者の操作は、環境変数 JWE_MOCK_SCRIPT のファイルから 1 行ずつ読み、
 * イベントが空のときに jwe_wait が出す:
 *   chrome <文字列>   … JWE_EV_CHROME_MESSAGE（外枠からの文字列）
 *   key <番号>        … JWE_EV_KEY
 *   newtab <URL>      … JWE_EV_NEW_TAB_REQUESTED（いま見えているタブから）
 *   close             … JWE_EV_CLOSE_REQUESTED
 * ファイルが終わったら CLOSE_REQUESTED を 1 回出す。JWE_MOCK_LOG を指定すると、呼ばれた関数を書き出す。
 */
#include "../jw_engine.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    char **urls;
    uint32_t len, cap, pos; /* pos: いまの位置（len が 0 なら空） */
} History;

typedef struct {
    uint32_t id;
    int open;
    History h;
} Tab;

typedef struct {
    JweEvent ev;
    char *owned;
} Queued;

struct JweEngine {
    Tab *tabs;
    uint32_t ntabs, cap_tabs, shown;
    Queued *q;
    uint32_t qhead, qlen, qcap;
    char *last; /* 最後に渡したイベントの文字列（次の poll で解放） */
    FILE *script;
    FILE *log;
    int close_sent;
    int chrome;
};

static char *dup_n(const char *s, uint32_t n) {
    char *r = (char *)malloc((size_t)n + 1);
    if (!r) abort();
    memcpy(r, s, n);
    r[n] = 0;
    return r;
}

static void logf_(JweEngine *e, const char *what, uint32_t tab, const char *s, uint32_t n) {
    if (!e->log) return;
    fprintf(e->log, "%s %u", what, tab);
    if (s) fprintf(e->log, " %.*s", (int)n, s);
    fputc('\n', e->log);
    fflush(e->log);
}

static void push(JweEngine *e, uint32_t kind, uint32_t tab, int64_t code, const char *text, uint32_t len) {
    if (e->qlen == e->qcap) {
        uint32_t nc = e->qcap ? e->qcap * 2 : 16;
        Queued *nq = (Queued *)malloc(sizeof(Queued) * nc);
        if (!nq) abort();
        for (uint32_t i = 0; i < e->qlen; i++) nq[i] = e->q[(e->qhead + i) % e->qcap];
        free(e->q);
        e->q = nq;
        e->qcap = nc;
        e->qhead = 0;
    }
    Queued *x = &e->q[(e->qhead + e->qlen) % e->qcap];
    x->owned = text ? dup_n(text, len) : NULL;
    x->ev.kind = kind;
    x->ev.tab = tab;
    x->ev.code = code;
    x->ev.text = x->owned;
    x->ev.text_len = text ? len : 0;
    e->qlen++;
}

static int pop(JweEngine *e, JweEvent *out) {
    free(e->last);
    e->last = NULL;
    if (e->qlen == 0) return 0;
    Queued *x = &e->q[e->qhead];
    e->qhead = (e->qhead + 1) % e->qcap;
    e->qlen--;
    *out = x->ev;
    e->last = x->owned;
    return 1;
}

static Tab *find(JweEngine *e, uint32_t id) {
    for (uint32_t i = 0; i < e->ntabs; i++)
        if (e->tabs[i].id == id && e->tabs[i].open) return &e->tabs[i];
    return NULL;
}

static void history_free(History *h) {
    for (uint32_t i = 0; i < h->len; i++) free(h->urls[i]);
    free(h->urls);
    memset(h, 0, sizeof *h);
}

static void history_state(JweEngine *e, Tab *t) {
    int64_t c = 0;
    if (t->h.len && t->h.pos > 0) c |= 1;
    if (t->h.len && t->h.pos + 1 < t->h.len) c |= 2;
    push(e, JWE_EV_HISTORY_CHANGED, t->id, c, NULL, 0);
}

static void load(JweEngine *e, Tab *t, const char *url) {
    uint32_t n = (uint32_t)strlen(url);
    char title[1024];
    int tn = snprintf(title, sizeof title, "title:%s", url);
    if (tn < 0) tn = 0;
    if (tn >= (int)sizeof title) tn = (int)sizeof title - 1;
    push(e, JWE_EV_NAV_STARTED, t->id, 0, url, n);
    push(e, JWE_EV_URL_CHANGED, t->id, 0, url, n);
    push(e, JWE_EV_TITLE_CHANGED, t->id, 0, title, (uint32_t)tn);
    /* "fail:" で始まる URL は読み込みに失敗したことにする */
    push(e, JWE_EV_NAV_FINISHED, t->id, strncmp(url, "fail:", 5) == 0 ? 1 : 0, url, n);
    history_state(e, t);
}

JweStatus jwe_open(const JweConfig *cfg, JweEngine **out) {
    if (!cfg || !out) return JWE_ERR_BAD_ARG;
    JweEngine *e = (JweEngine *)calloc(1, sizeof *e);
    if (!e) return JWE_ERR_FAILED;
    const char *s = getenv("JWE_MOCK_SCRIPT");
    if (s && *s) e->script = fopen(s, "r");
    const char *l = getenv("JWE_MOCK_LOG");
    if (l && *l) e->log = fopen(l, "wb");
    logf_(e, "open", 0, cfg->title, cfg->title_len);
    push(e, JWE_EV_READY, 0, 0, NULL, 0);
    *out = e;
    return JWE_OK;
}

void jwe_close(JweEngine *e) {
    if (!e) return;
    logf_(e, "close", 0, NULL, 0);
    for (uint32_t i = 0; i < e->ntabs; i++) history_free(&e->tabs[i].h);
    free(e->tabs);
    JweEvent ev;
    while (pop(e, &ev)) {}
    free(e->last);
    free(e->q);
    if (e->script) fclose(e->script);
    if (e->log) fclose(e->log);
    free(e);
}

void jwe_info(const JweEngine *e, JweInfo *out) {
    (void)e;
    out->abi_version = JWE_ABI_VERSION;
    out->caps = JWE_CAP_CHROME_VIEW;
    out->name = "mock";
    out->version = "1";
}

int jwe_poll(JweEngine *e, JweEvent *out) { return pop(e, out); }

/* 台本の 1 行をイベントにする。台本が終わったら 0 */
static int script_step(JweEngine *e) {
    char line[4096];
    while (e->script && fgets(line, sizeof line, e->script)) {
        size_t n = strlen(line);
        while (n && (line[n - 1] == '\n' || line[n - 1] == '\r')) line[--n] = 0;
        if (n == 0 || line[0] == '#') continue;
        if (strncmp(line, "chrome ", 7) == 0) {
            push(e, JWE_EV_CHROME_MESSAGE, 0, 0, line + 7, (uint32_t)(n - 7));
        } else if (strncmp(line, "key ", 4) == 0) {
            push(e, JWE_EV_KEY, 0, atoi(line + 4), NULL, 0);
        } else if (strncmp(line, "newtab ", 7) == 0) {
            push(e, JWE_EV_NEW_TAB_REQUESTED, e->shown, 0, line + 7, (uint32_t)(n - 7));
        } else if (strcmp(line, "close") == 0) {
            push(e, JWE_EV_CLOSE_REQUESTED, 0, 0, NULL, 0);
        } else {
            push(e, JWE_EV_ERROR, 0, JWE_ERR_BAD_ARG, line, (uint32_t)n);
        }
        return 1;
    }
    return 0;
}

int jwe_wait(JweEngine *e, int32_t timeout_ms, JweEvent *out) {
    (void)timeout_ms;
    if (e->qlen == 0 && !script_step(e) && !e->close_sent) {
        e->close_sent = 1;
        push(e, JWE_EV_CLOSE_REQUESTED, 0, 0, NULL, 0);
    }
    return pop(e, out);
}

JweStatus jwe_tab_new(JweEngine *e, uint32_t *out_tab) {
    if (e->ntabs == e->cap_tabs) {
        uint32_t nc = e->cap_tabs ? e->cap_tabs * 2 : 8;
        Tab *nt = (Tab *)realloc(e->tabs, sizeof(Tab) * nc);
        if (!nt) return JWE_ERR_FAILED;
        e->tabs = nt;
        e->cap_tabs = nc;
    }
    Tab *t = &e->tabs[e->ntabs];
    memset(t, 0, sizeof *t);
    t->id = ++e->ntabs;
    t->open = 1;
    *out_tab = t->id;
    logf_(e, "tab_new", t->id, NULL, 0);
    return JWE_OK;
}

JweStatus jwe_tab_close(JweEngine *e, uint32_t tab) {
    Tab *t = find(e, tab);
    if (!t) return JWE_ERR_BAD_TAB;
    logf_(e, "tab_close", tab, NULL, 0);
    history_free(&t->h);
    t->open = 0;
    if (e->shown == tab) e->shown = 0;
    return JWE_OK;
}

JweStatus jwe_tab_show(JweEngine *e, uint32_t tab) {
    if (!find(e, tab)) return JWE_ERR_BAD_TAB;
    logf_(e, "tab_show", tab, NULL, 0);
    e->shown = tab;
    return JWE_OK;
}

JweStatus jwe_navigate(JweEngine *e, uint32_t tab, const char *url, uint32_t url_len) {
    Tab *t = find(e, tab);
    if (!t) return JWE_ERR_BAD_TAB;
    if (!url || url_len == 0) return JWE_ERR_BAD_ARG;
    logf_(e, "navigate", tab, url, url_len);
    History *h = &t->h;
    /* 今より先の履歴は捨てる */
    if (h->len) {
        for (uint32_t i = h->pos + 1; i < h->len; i++) free(h->urls[i]);
        h->len = h->pos + 1;
    }
    if (h->len == h->cap) {
        uint32_t nc = h->cap ? h->cap * 2 : 8;
        char **nu = (char **)realloc(h->urls, sizeof(char *) * nc);
        if (!nu) return JWE_ERR_FAILED;
        h->urls = nu;
        h->cap = nc;
    }
    h->urls[h->len++] = dup_n(url, url_len);
    h->pos = h->len - 1;
    load(e, t, h->urls[h->pos]);
    return JWE_OK;
}

JweStatus jwe_back(JweEngine *e, uint32_t tab) {
    Tab *t = find(e, tab);
    if (!t) return JWE_ERR_BAD_TAB;
    logf_(e, "back", tab, NULL, 0);
    if (t->h.len && t->h.pos > 0) load(e, t, t->h.urls[--t->h.pos]);
    return JWE_OK;
}

JweStatus jwe_forward(JweEngine *e, uint32_t tab) {
    Tab *t = find(e, tab);
    if (!t) return JWE_ERR_BAD_TAB;
    logf_(e, "forward", tab, NULL, 0);
    if (t->h.len && t->h.pos + 1 < t->h.len) load(e, t, t->h.urls[++t->h.pos]);
    return JWE_OK;
}

JweStatus jwe_reload(JweEngine *e, uint32_t tab) {
    Tab *t = find(e, tab);
    if (!t) return JWE_ERR_BAD_TAB;
    logf_(e, "reload", tab, NULL, 0);
    if (t->h.len) load(e, t, t->h.urls[t->h.pos]);
    return JWE_OK;
}

JweStatus jwe_stop(JweEngine *e, uint32_t tab) {
    if (!find(e, tab)) return JWE_ERR_BAD_TAB;
    logf_(e, "stop", tab, NULL, 0);
    return JWE_OK;
}

JweStatus jwe_chrome_load(JweEngine *e, const char *html, uint32_t html_len, uint32_t height) {
    (void)html;
    logf_(e, "chrome_load", height, NULL, html_len);
    e->chrome = 1;
    return JWE_OK;
}

JweStatus jwe_chrome_post(JweEngine *e, const char *msg, uint32_t msg_len) {
    if (!e->chrome) return JWE_ERR_FAILED;
    logf_(e, "chrome_post", 0, msg, msg_len);
    return JWE_OK;
}

JweStatus jwe_set_title(JweEngine *e, const char *title, uint32_t title_len) {
    logf_(e, "set_title", 0, title, title_len);
    return JWE_OK;
}

JweStatus jwe_clear_site_data(JweEngine *e) {
    logf_(e, "clear_site_data", 0, NULL, 0);
    return JWE_OK;
}
