// jabberwock の実行時ライブラリ。`jabberwock build` が LLVM の書き出し（src/ir/llvm.rs）と一緒に clang でコンパイルする。
//
// - own の先は「区画」。区画の頭（8 バイト）に世代を置く。解放しても OS には返さず、
//   同じ大きさの区画の置き場に戻すだけ（世代は進めたまま残るので、古い ref は必ず見つかる）。
// - ref は (指す先, 区画, 借りたときの世代, 借りた場所)。区画の世代と比べて確かめる（比べるのは書き出したコード）。
// - own の「読むだけの写し」（for の要素・クロージャが取り込んだ値）は、下の 1 ビットを立てたポインタで、
//   先は「写しの札」（区画, 世代, 借りた場所）。先をたどるときに世代を確かめる。
// - 文字列は書き換えない。数を数えて共有する（rc が負なら消えない定数）。
// - 実行時エラーは、出力をすべて書き出してから標準エラーに出し、終了コード 1 で終わる。

// -std=c11 でも getaddrinfo などの POSIX の宣言が見えるように（glibc。macOS は何もしなくても見える）
#if !defined(_WIN32) && !defined(_DEFAULT_SOURCE)
#define _DEFAULT_SOURCE
#endif
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// All runtime allocation failures are language errors, including metadata and output buffers.
static void *jw_checked_malloc(size_t n);
static void *jw_checked_calloc(size_t n, size_t size);
static void *jw_checked_realloc(void *old, size_t n);

// LLVM の計測版から呼ぶ。標準エラーの FILE は C 側で扱い、Windows でも同じ表示にする。
void jw_report_checks(int64_t gen, int64_t elided, int64_t bounds) {
    fprintf(stderr, "[checks] gen %lld elided %lld bounds %lld\n",
            (long long)gen, (long long)elided, (long long)bounds);
}

// ---------------------------------------------------------------- スレッドの道具（錠と条件変数）

#ifdef _WIN32
// windows.h なしで宣言する（SRWLOCK と CONDITION_VARIABLE はポインタ 1 つ分。0 で初期化）
typedef struct {
    void *p;
} WLock;
typedef struct {
    void *p;
} WCond;
#define WLOCK_INIT {0}
#define WCOND_INIT {0}
__declspec(dllimport) void __stdcall AcquireSRWLockExclusive(WLock *);
__declspec(dllimport) void __stdcall ReleaseSRWLockExclusive(WLock *);
__declspec(dllimport) int __stdcall SleepConditionVariableSRW(WCond *, WLock *, unsigned long, unsigned long);
__declspec(dllimport) void __stdcall WakeAllConditionVariable(WCond *);
__declspec(dllimport) void *__stdcall CreateThread(void *, size_t, unsigned long(__stdcall *)(void *), void *, unsigned long, unsigned long *);
__declspec(dllimport) unsigned long __stdcall WaitForSingleObject(void *, unsigned long);
__declspec(dllimport) int __stdcall CloseHandle(void *);
static void lock(WLock *l) { AcquireSRWLockExclusive(l); }
static void unlock(WLock *l) { ReleaseSRWLockExclusive(l); }
static void cond_wait(WCond *c, WLock *l) { SleepConditionVariableSRW(c, l, 0xFFFFFFFFul, 0); }
static void cond_broadcast(WCond *c) { WakeAllConditionVariable(c); }
typedef void *WThread;
static unsigned long __stdcall thread_start(void *arg);
static int thread_new(WThread *t, void *arg) {
    *t = CreateThread(NULL, (size_t)256 << 20, thread_start, arg, 0x00010000 /* STACK_SIZE_PARAM_IS_A_RESERVATION */, NULL);
    return *t != NULL;
}
static void thread_join(WThread t) {
    WaitForSingleObject(t, 0xFFFFFFFFul);
    CloseHandle(t);
}
#else
#include <pthread.h>
typedef pthread_mutex_t WLock;
typedef pthread_cond_t WCond;
#define WLOCK_INIT PTHREAD_MUTEX_INITIALIZER
#define WCOND_INIT PTHREAD_COND_INITIALIZER
static void lock(WLock *l) { pthread_mutex_lock(l); }
static void unlock(WLock *l) { pthread_mutex_unlock(l); }
static void cond_wait(WCond *c, WLock *l) { pthread_cond_wait(c, l); }
static void cond_broadcast(WCond *c) { pthread_cond_broadcast(c); }
typedef pthread_t WThread;
static void *thread_start(void *arg);
static int thread_new(WThread *t, void *arg) {
    pthread_attr_t a;
    pthread_attr_init(&a);
    pthread_attr_setstacksize(&a, (size_t)256 << 20);
    int r = pthread_create(t, &a, thread_start, arg);
    pthread_attr_destroy(&a);
    return r == 0;
}
static void thread_join(WThread t) { pthread_join(t, NULL); }
#endif

// spawn したことがあるか（なければ出力の錠を取らない）。最初の spawn の前に立てる。
// 複数スレッドで spawn と出力が重なるため、同じ 1 の書き込みでも不可分の操作にする。
static int g_threaded;

// 位置（ファイル・行・列・長さ）と、その位置の説明。解放の場所なら chain は「連鎖して解放された」の位置
typedef struct WSite {
    const char *file;
    int32_t line, col, len, fileno;
    const char *text;
    const struct WSite *chain;
} WSite;

typedef struct WStr {
    int64_t rc;
    int64_t len;
    char data[];
} WStr;

typedef struct WList {
    char *data;
    int64_t len, cap;
} WList;

typedef struct WSb {
    char *p;
    int64_t len, cap;
    char inline_buf[48];
} WSb;

// ---------------------------------------------------------------- 出力

static char *out_buf;
static size_t out_len, out_cap;
static WLock out_lock = WLOCK_INIT;
static _Thread_local int out_owned;
static void out_enter(void) {
    if (__atomic_load_n(&g_threaded, __ATOMIC_RELAXED)) { lock(&out_lock); out_owned = 1; }
}
static void out_leave(void) {
    if (out_owned) { out_owned = 0; unlock(&out_lock); }
}

static void flush_locked(void) {
    if (out_len) fwrite(out_buf, 1, out_len, stdout);
    out_len = 0;
    fflush(stdout);
}

void jw_flush(void) {
    // Allocation failure may occur while this thread is assembling a print line.
    if (out_owned) { flush_locked(); return; }
    out_enter();
    flush_locked();
    out_leave();
}

// エラーと exit で終わる: 片づけずに終わる（atexit の処理も動かさない。ほかのスレッドがまだ動いていても
// 後始末と競合しない。LeakSanitizer も、片づけない終わり方を漏れと見なさない）
static _Noreturn void quit(int code) {
    fflush(stdout);
    fflush(stderr);
    _Exit(code);
}

static void out_write(const char *p, size_t n) {
    if (out_len + n > out_cap) {
        size_t cap = out_cap ? out_cap * 2 : 1 << 16;
        while (cap < out_len + n) cap *= 2;
        out_buf = jw_checked_realloc(out_buf, cap);
        out_cap = cap;
    }
    memcpy(out_buf + out_len, p, n);
    out_len += n;
    if (out_len >= 1 << 16) flush_locked();
}

// 1 回の print のあいだ出力の錠を持つ（行が混ざらない）
void jw_print_begin(void) {
    out_enter();
}

// print の 1 つ分（first でなければ前に空白）
void jw_print_part(WStr *s, int32_t first) {
    if (!first) out_write(" ", 1);
    out_write(s->data, (size_t)s->len);
}

void jw_print_end(void) {
    out_write("\n", 1);
    out_leave();
}

// eprint: 標準エラーへ 1 行。先に標準出力を出しておく（端末で順番が入れ替わらないように）
void jw_eprint_begin(void) {
    out_enter();
    flush_locked();
}

void jw_eprint_part(WStr *s, int32_t first) {
    if (!first) fputc(' ', stderr);
    fwrite(s->data, 1, (size_t)s->len, stderr);
}

void jw_eprint_end(void) {
    fputc('\n', stderr);
    fflush(stderr);
    out_leave();
}

// ---------------------------------------------------------------- エラー

static int is_cont_b(unsigned char c) { return (c & 0xC0) == 0x80; }

// 書き出したコードが置く: ファイルの名前と中身（エラーでソースの行を出すため。`--release` では空）
typedef struct WSrc {
    const char *name;
    const char *text;
} WSrc;
extern const WSrc jw_files[];
extern const int32_t jw_nfiles;

// エラーの位置を集めてから、`Diagnostic::render_in` と同じ形で出す
#define MAXL 8
static struct {
    const WSite *s;
    char text[600];
    int primary;
} labels[MAXL];
static int nlabels;
static char err_title[1024];

static WLock err_lock = WLOCK_INIT;

static void begin_error(const char *title) {
    // エラーを出すのは 1 つのスレッドだけ（出したら終わる。錠は返さない）
    if (__atomic_load_n(&g_threaded, __ATOMIC_RELAXED)) lock(&err_lock);
    jw_flush();
    snprintf(err_title, sizeof err_title, "%s", title);
    nlabels = 0;
}

static void put_site(const WSite *s, const char *label) {
    if (nlabels == MAXL) return;
    labels[nlabels].s = s;
    snprintf(labels[nlabels].text, sizeof labels[nlabels].text, "%s", label ? label : "");
    labels[nlabels].primary = 0;
    nlabels++;
}

// ソースの ln 行目（1 始まり）。Rust の `lines()` と同じ区切り
static const char *line_of(const char *src, int ln, int64_t *len) {
    const char *p = src;
    for (int i = 1; i < ln && *p; i++) {
        const char *nl = strchr(p, '\n');
        p = nl ? nl + 1 : p + strlen(p);
    }
    const char *e = strchr(p, '\n');
    int64_t n = e ? (int64_t)(e - p) : (int64_t)strlen(p);
    if (n > 0 && p[n - 1] == '\r') n--;
    *len = n;
    return p;
}

static void spaces(int n) {
    for (int i = 0; i < n; i++) fputc(' ', stderr);
}

static void arrow(int width, const WSite *s) {
    spaces(width);
    fprintf(stderr, "--> %s:%d:%d\n", s->file, s->line, s->col);
    spaces(width);
    fprintf(stderr, " |\n");
}

static int label_after(const WSite *a, const WSite *b, int main_file) {
    int ka = a->fileno != main_file, kb = b->fileno != main_file;
    if (ka != kb) return ka > kb;
    if (a->fileno != b->fileno) return a->fileno > b->fileno;
    if (a->line != b->line) return a->line > b->line;
    return a->col > b->col;
}

static _Noreturn void finish_error(const WSite *at, const char *label, const char *note) {
    put_site(at, label);
    labels[nlabels - 1].primary = 1;
    fprintf(stderr, "error: %s\n", err_title);
    if (jw_nfiles == 0) {
        // --release: 使った位置だけ
        fprintf(stderr, " --> %s:%d:%d\n", at->file, at->line, at->col);
        fprintf(stderr, "  = %s\n", label ? label : "");
        quit(1);
    }
    int order[MAXL], main_file = at->fileno;
    for (int i = 0; i < nlabels; i++) order[i] = i;
    for (int i = 1; i < nlabels; i++)
        for (int j = i; j > 0 && label_after(labels[order[j - 1]].s, labels[order[j]].s, main_file); j--) {
            int t = order[j];
            order[j] = order[j - 1];
            order[j - 1] = t;
        }
    int maxline = 1;
    for (int i = 0; i < nlabels; i++)
        if (labels[i].s->line > maxline) maxline = labels[i].s->line;
    char tmp[16];
    int width = snprintf(tmp, sizeof tmp, "%d", maxline);
    arrow(width, at);
    int last = 0, cur = main_file;
    for (int k = 0; k < nlabels; k++) {
        const WSite *s = labels[order[k]].s;
        if (s->fileno != cur) {
            // 別のファイルの位置
            cur = s->fileno;
            last = 0;
            arrow(width, s);
        }
        const char *src = cur < jw_nfiles ? jw_files[cur].text : jw_files[0].text;
        int64_t n;
        const char *line = line_of(src, s->line, &n);
        if (s->line != last) {
            if (last != 0 && s->line > last + 1) {
                spaces(width);
                fprintf(stderr, " ...\n");
            }
            fprintf(stderr, "%*d | ", width, s->line);
            for (int64_t i = 0; i < n; i++) fputc(line[i] == '\t' ? ' ' : line[i], stderr);
            fputc('\n', stderr);
            last = s->line;
        }
        // 下線: 列は文字単位。ASCII でない文字は 2 桁
        int lead = 0, mark = 0, col = 1, upto = s->col + (s->len > 0 ? s->len : 1);
        for (int64_t i = 0; i < n; col++) {
            int w = (unsigned char)line[i] < 0x80 ? 1 : 2;
            if (col < s->col) lead += w;
            else if (col < upto) mark += w;
            i++;
            while (i < n && is_cont_b((unsigned char)line[i])) i++;
        }
        if (mark < 1) mark = 1;
        spaces(width);
        fprintf(stderr, " | ");
        spaces(lead);
        for (int m = 0; m < mark; m++) fputc(labels[order[k]].primary ? '^' : '-', stderr);
        fprintf(stderr, " %s\n", labels[order[k]].text);
    }
    if (note) {
        spaces(width);
        fprintf(stderr, " |\n");
        spaces(width);
        fprintf(stderr, " = note: %s\n", note);
    }
    quit(1);
}

_Noreturn void jw_fail(const char *title, const WSite *at, const char *label, const char *note) {
    begin_error(title);
    finish_error(at, label, note);
}

// 確保失敗は内部エラーではなく、言語仕様上の実行時エラー。
static _Noreturn void jw_oom(void) {
    jw_flush();
    fputs("error: メモリが足りません\n", stderr);
    quit(1);
}

static void *jw_checked_malloc(size_t n) {
    void *p = malloc(n ? n : 1);
    if (!p) jw_oom();
    return p;
}
static void *jw_checked_calloc(size_t n, size_t size) {
    if (size && n > SIZE_MAX / size) jw_oom();
    void *p = calloc(n && size ? n : 1, n && size ? size : 1);
    if (!p) jw_oom();
    return p;
}
static void *jw_checked_realloc(void *old, size_t n) {
    void *p = realloc(old, n ? n : 1);
    if (!p) jw_oom();
    return p;
}

// IR では来ないはずの場所（書き出しの誤り）
_Noreturn void jw_bug(const char *what) {
    jw_flush();
    fprintf(stderr, "内部エラー: %s\n", what);
    quit(2);
}

static int same_site(const WSite *a, const WSite *b) {
    return a->line == b->line && a->col == b->col && a->len == b->len && strcmp(a->file, b->file) == 0;
}

_Noreturn void jw_fail_moved(const char *name, const WSite *moved, const WSite *use) {
    char title[512], label[512];
    snprintf(title, sizeof title, "ムーブ済みの値 `%s` を使いました (use after move)", name);
    begin_error(title);
    if (same_site(moved, use)) {
        snprintf(label, sizeof label, "ここで `%s` の所有権を移すが、前の周回ですでに移動済み", name);
        finish_error(use, label, "ループの中で own を渡すと、2周目には手元に残っていません");
    }
    snprintf(label, sizeof label, "ここで `%s` の所有権が移動した", name);
    put_site(moved, label);
    snprintf(label, sizeof label, "ここで `%s` を使ったが、もう所有していない", name);
    finish_error(use, label, "移した後も使いたいなら `ref` で借りてください（例: `f(ref a)`）");
}

// 区画の頭は世代だけ（8 バイト）。世代が奇数なら解放済みで、中身の 2 語目に解放した位置を置く（1 語目は空き区画の鎖）
typedef struct WHdr {
    uint64_t gen;
} WHdr;

#define HDR(p) ((WHdr *)((char *)(p) - sizeof(WHdr)))
#define FREED_SITE(p) (((const WSite **)(p))[1])

// 所有者だけが世代を書き換えるが、送信元の無効な ref が別のスレッドで読むことはある。
// 読み書きは不可分にする。書き手は唯一なので、加算も load/store でよい（read-modify-write は不要）。
static uint64_t gen_get(void *p) { return __atomic_load_n(&HDR(p)->gen, __ATOMIC_RELAXED); }
static void gen_set(void *p, uint64_t gen) { __atomic_store_n(&HDR(p)->gen, gen, __ATOMIC_RELAXED); }

// bit62はリストの保持印、bit0/1は解放/借用印。bit63を含む全1は退役専用で、refには発行しない。
#define GEN_RETIRED UINT64_MAX
#define GEN_COUNTER_MASK ((UINT64_C(1) << 62) - 1)
#ifdef JW_TEST_GENERATION_LIMIT
_Static_assert(JW_TEST_GENERATION_LIMIT >= 3 && JW_TEST_GENERATION_LIMIT <= GEN_COUNTER_MASK
               && (JW_TEST_GENERATION_LIMIT & 3) == 3,
               "test generation limit must include both low state bits");
#define GEN_LIMIT ((uint64_t)JW_TEST_GENERATION_LIMIT)
#else
#define GEN_LIMIT GEN_COUNTER_MASK
#endif
// live payloadとは別に、全スレッドの退役識別子を数える。
static uint64_t retired_blocks;
static void *retired_head;
static uint64_t retired_count(void) { return __atomic_load_n(&retired_blocks, __ATOMIC_RELAXED); }

static int gen_has_room(uint64_t gen, uint64_t step) {
    return gen != GEN_RETIRED && step <= GEN_LIMIT && (gen & GEN_COUNTER_MASK) <= GEN_LIMIT - step;
}

static void gen_retire(void *p) {
    if (gen_get(p) != GEN_RETIRED) {
        gen_set(p, GEN_RETIRED);
        // 退役後の先頭語は保持表のリンクに転用する。古いrefはpayloadを読む前に拒否する。
        // 保持表はプロセス終了まで削除しない。別スレッドの退役も安全に公開する。
        void *head = __atomic_load_n(&retired_head, __ATOMIC_RELAXED);
        do { *(void **)p = head; }
        while (!__atomic_compare_exchange_n(&retired_head, &head, p, 0, __ATOMIC_RELEASE, __ATOMIC_RELAXED));
        __atomic_fetch_add(&retired_blocks, 1, __ATOMIC_RELAXED);
    }
}

// 所有者だけが書く。上限確認からstoreの間にも過去の世代値を公開しない。
static int gen_advance(void *p, uint64_t step) {
    uint64_t gen = gen_get(p);
    if (!gen_has_room(gen, step)) {
        gen_retire(p);
        return 0;
    }
    gen_set(p, gen + step);
    return 1;
}

// 使い回した区画の、前の解放の位置（エラーで「ここで解放された」を出すため。古いものから消える）
#define NREUSED 4096
static _Thread_local struct {
    void *block;
    uint64_t gen;
    const WSite *freed;
} reused[NREUSED];
static _Thread_local unsigned reused_n;

// ほかのスレッドへ送った区画と、要素が動いたリストの中身の区画（世代が進むが中身は使われ続けるので、位置はここに残す）。
// gen は進める前の世代（それを持つ ref を無効にした出来事）
static _Thread_local struct {
    void *block;
    uint64_t gen;
    const WSite *why;
} sent[NREUSED];
static _Thread_local unsigned sent_n;

static void remember_sent(void *p, const WSite *why) {
    unsigned i = sent_n++ % NREUSED;
    sent[i].block = p;
    sent[i].gen = gen_get(p);
    sent[i].why = why;
}

// 世代 gen の ref を無効にした出来事（解放なら世代 gen+1、送った・要素が動いたなら表の中）の位置
static const WSite *freed_at(void *block, uint64_t gen) {
    if (gen == GEN_RETIRED) return NULL;
    // 退役したpayloadは診断用にも読まない。保持したmetadata/スレッドローカル履歴だけを使う。
    uint64_t current = gen_get(block);
    if (current != GEN_RETIRED && current == gen + 1) return FREED_SITE(block);
    for (unsigned k = 0; k < NREUSED; k++) {
        unsigned i = (reused_n - 1 - k) % NREUSED;
        if (reused[i].block == block && reused[i].gen == gen + 1) return reused[i].freed;
    }
    for (unsigned k = 0; k < NREUSED && k < sent_n; k++) {
        unsigned i = (sent_n - 1 - k) % NREUSED;
        if (sent[i].block == block && sent[i].gen == gen) return sent[i].why;
    }
    return NULL;
}

_Noreturn void jw_fail_dangling(const WSite *use, const WSite *borrowed, void *block, uint64_t gen) {
    char label[512];
    begin_error("無効になった参照を使いました (dangling reference)");
    snprintf(label, sizeof label, "ここで `%s` を借りた", borrowed->text);
    put_site(borrowed, label);
    const WSite *f = freed_at(block, gen);
    if (f) {
        snprintf(label, sizeof label, "ここで解放された: %s", f->text);
        put_site(f, label);
    }
    finish_error(use, "ここで使ったが、借りた先はもう存在しない", "参照は、借りた元の所有者より長くは生きられません");
}

static _Noreturn void fail_generation(const WSite *at) {
    begin_error("世代番号の上限に達しました");
    finish_error(at, "これ以上この区画の参照を安全に無効化できません", NULL);
}

// 整数を 10 進の文字にする（buf は 21 バイト以上）。書いた長さを返す
static int fmt_int(char *buf, size_t n, int64_t v, int32_t is_unsigned) {
    (void)n;
    char tmp[24];
    int k = 0;
    int neg = !is_unsigned && v < 0;
    uint64_t u = neg ? 0 - (uint64_t)v : (uint64_t)v;
    do {
        tmp[k++] = (char)('0' + u % 10);
        u /= 10;
    } while (u);
    int len = 0;
    if (neg) buf[len++] = '-';
    while (k) buf[len++] = tmp[--k];
    buf[len] = 0;
    return len;
}

_Noreturn void jw_fail_bounds(int64_t i, int32_t is_unsigned, int64_t len, const WSite *at) {
    char title[256], label[256], num[32];
    fmt_int(num, sizeof num, i, is_unsigned);
    snprintf(title, sizeof title, "添字 %s は配列の範囲の外です", num);
    snprintf(label, sizeof label, "長さは %lld なので、添字は 0〜%lld です", (long long)len, (long long)len - 1);
    begin_error(title);
    finish_error(at, label, NULL);
}

_Noreturn void jw_fail_depth(const WSite *at) {
    begin_error("関数呼び出しが深すぎます");
    finish_error(at, "呼び出しの深さが 5000 を超えました", NULL);
}

// 辞書にキーがない（kind: 0 整数・1 文字列・2 真偽値）
_Noreturn void jw_fail_nokey(int32_t kind, int64_t key, int32_t is_unsigned, const WSite *at) {
    char title[1024], shown[900];
    if (kind == 1) {
        WStr *s = (WStr *)(intptr_t)key;
        snprintf(shown, sizeof shown, "\"%.*s\"", (int)(s->len > 800 ? 800 : s->len), s->data);
    } else if (kind == 2) {
        snprintf(shown, sizeof shown, "%s", key ? "true" : "false");
    } else {
        fmt_int(shown, sizeof shown, key, is_unsigned);
    }
    snprintf(title, sizeof title, "辞書にキー %s がありません", shown);
    begin_error(title);
    finish_error(at, "このキーの値は入っていない", "ないかもしれないなら `m.get(k)`（Option）か `m.contains(k)` を使ってください");
}

_Noreturn void jw_fail_insert(int64_t i, int64_t len, const WSite *at) {
    char label[256];
    snprintf(label, sizeof label, "位置は %lld ですが、長さは %lld です", (long long)i, (long long)len);
    begin_error("insert の位置が範囲の外です");
    finish_error(at, label, "入れられる位置は 0〜長さ です");
}

// ---------------------------------------------------------------- 区画

// 生きている区画の数と呼び出しの深さは、スレッドごと（区画は別のスレッドで解放されうるので、数は全スレッドの合計で見る）
_Thread_local int64_t jw_live;
_Thread_local int64_t jw_depth;

// 小さい区画は 8 バイト刻みの大きさごとに空き区画の鎖を持ち、まとめて確保した塊から切り出す（OS には返さない）
#define NCLASS 256
static _Thread_local void *free_small[NCLASS + 1];
typedef struct Big {
    struct Big *next;
    size_t size;
    void *payload;
} Big;
static _Thread_local Big *free_big;
static _Thread_local char *slab_cur, *slab_end;

static size_t round8(int64_t n) { return n <= 16 ? 16 : ((size_t)n + 7) & ~(size_t)7; }

// 塊から n バイト（8 の倍数）を切り出す
static void *slab_take(size_t n) {
    const size_t slab_size = 256 * 1024;
    // Large requests get their own process-lifetime allocation. Keep the
    // arena subtraction below restricted to initialized pointers from one slab.
    if (n > slab_size) {
        void *r = jw_checked_malloc(n);
        if (!r) jw_oom();
        return r;
    }
    if (!slab_cur || !slab_end || (size_t)(slab_end - slab_cur) < n) {
        slab_cur = jw_checked_malloc(slab_size);
        if (!slab_cur) jw_oom();
        slab_end = slab_cur + slab_size;
    }
    void *r = slab_cur;
    slab_cur += n;
    return r;
}

static int remember_reuse(void *p) {
    if (!gen_has_room(gen_get(p), 1)) {
        gen_retire(p);
        return 0;
    }
    unsigned i = reused_n++ % NREUSED;
    reused[i].block = p;
    reused[i].gen = gen_get(p);
    reused[i].freed = FREED_SITE(p);
    return gen_advance(p, 1);
}

// 区画を確保する（生きている区画の数は数えない。リストの中身の区画はこれだけを使う）
static void *block_alloc(int64_t size) {
    size_t s = round8(size);
    void *p = NULL;
    if (s / 8 <= NCLASS) {
        while ((p = free_small[s / 8]) != NULL) {
            free_small[s / 8] = *(void **)p;
            if (remember_reuse(p)) break;
        }
        if (!p) {
            WHdr *h = slab_take(sizeof(WHdr) + s);
            gen_set(h + 1, 0);
            p = h + 1;
        }
    } else {
        for (Big **b = &free_big; *b;) {
            if ((*b)->size == s) {
                Big *x = *b;
                *b = x->next;
                p = x->payload;
                free(x);
                if (remember_reuse(p)) break;
                p = NULL;
            } else b = &(*b)->next;
        }
        if (!p) {
            // 中身を 64 バイトの境目にそろえる（リストの要素をまとめて読み書きする命令が速くなる）。大きな区画は OS に返さない
            char *raw = jw_checked_malloc(sizeof(WHdr) + s + 64);
            if (!raw) jw_oom();
            p = (void *)(((uintptr_t)raw + sizeof(WHdr) + 63) & ~(uintptr_t)63);
            gen_set(p, 0);
        }
    }
    return p;
}

void *jw_alloc(int64_t size) {
    jw_live++;
    return block_alloc(size);
}

// 区画を解放する（生きている区画の数は数えない）
static void block_free(void *p, int64_t size, const WSite *why) {
    // 解放+次の再利用の両方の余地がなければ、空き表へ戻さず早期退役する。
    if (!gen_has_room(gen_get(p), 2)) {
        remember_sent(p, why);
        gen_retire(p);
        return;
    }
    (void)gen_advance(p, 1);
    FREED_SITE(p) = why;
    size_t s = round8(size);
    if (s / 8 <= NCLASS) {
        *(void **)p = free_small[s / 8];
        free_small[s / 8] = p;
    } else {
        Big *b = jw_checked_malloc(sizeof(Big));
        b->next = free_big;
        b->size = s;
        b->payload = p;
        free_big = b;
    }
}

// 区画を解放する（中身の own は書き出したコードが先に解放する）。世代が進むので、ここを指す ref は無効になる
void jw_free(void *p, int64_t size, const WSite *why) {
    jw_live--;
    block_free(p, size, why);
}

uint64_t jw_gen(void *p) { return gen_get(p); }

// ---------------------------------------------------------------- own の読むだけの写し

typedef struct WCell {
    void *target;
    uint64_t gen;
    const WSite *site;
} WCell;

#define IS_VIEW(p) (((uintptr_t)(p)) & 1)
#define CELL(p) ((WCell *)(((uintptr_t)(p)) & ~(uintptr_t)1))

// own（または写し）の写しを作る。写しの写しは同じ先・同じ借りた場所
static _Thread_local WCell *free_cells;

void *jw_view(void *own, const WSite *site) {
    WCell *c = free_cells;
    if (c) free_cells = *(WCell **)c;
    else c = slab_take(sizeof(WCell));
    if (IS_VIEW(own)) {
        *c = *CELL(own);
    } else {
        c->target = own;
        c->gen = gen_get(own);
        c->site = site;
    }
    return (void *)((uintptr_t)c | 1);
}

void jw_view_free(void *v) {
    WCell *c = CELL(v);
    *(WCell **)c = free_cells;
    free_cells = c;
}

// 写しの先（世代を確かめる）
void *jw_view_target(void *v, const WSite *use) {
    WCell *c = CELL(v);
    uint64_t gen = gen_get(c->target);
    if (gen == GEN_RETIRED || gen != c->gen) jw_fail_dangling(use, c->site, c->target, c->gen);
    return c->target;
}

// 写しでなければ v のまま（書き出しは写しかどうかを見る前に呼んで select で選ぶことがある。own の中身を読まないように）
void *jw_view_target_nocheck(void *v) { return IS_VIEW(v) ? CELL(v)->target : v; }
uint64_t jw_view_gen(void *v) { return CELL(v)->gen; }
const WSite *jw_view_site(void *v) { return CELL(v)->site; }

// ---------------------------------------------------------------- 文字列

// 文字列と文字の組み立て用の小さい領域（16 バイト刻みの空き領域の鎖。区画と違って世代の頭はない）
#define SCLASS 64
static _Thread_local void *free_bytes[SCLASS + 1];

static void *bytes_alloc(size_t n) {
    size_t c = (n + 15) >> 4;
    if (c > SCLASS) {
        void *p = jw_checked_malloc(n);
        if (!p) jw_oom();
        return p;
    }
    void *p = free_bytes[c];
    if (p) {
        free_bytes[c] = *(void **)p;
        return p;
    }
    return slab_take(c << 4);
}

static void bytes_free(void *p, size_t n) {
    size_t c = (n + 15) >> 4;
    if (c > SCLASS) {
        free(p);
        return;
    }
    *(void **)p = free_bytes[c];
    free_bytes[c] = p;
}

// 文字列の区画の大きさは長さだけで決まる（解放のときも長さから求める）。
// 大きいもの（malloc）は 2 のべき乗に切り上げるので、後ろに足す（jw_str_append）ときに毎回は取り直さない
// 文字列の長さの上限（バイト）。str_bytes の切り上げがあふれないように（interp の STR_MAX と同じ）
#define STR_MAX ((int64_t)1 << 62)

static size_t str_bytes(int64_t len) {
    size_t n = sizeof(WStr) + (size_t)len + 1;
    if (((n + 15) >> 4) <= SCLASS) return n;
    size_t c = (size_t)SCLASS << 4;
    while (c < n) c <<= 1;
    return c;
}

static WStr *str_alloc(int64_t len) {
    WStr *s = bytes_alloc(str_bytes(len));
    s->rc = 1;
    s->len = len;
    s->data[len] = 0;
    return s;
}

WStr *jw_str_new(const char *p, int64_t n) {
    WStr *s = bytes_alloc(str_bytes(n));
    s->rc = 1;
    s->len = n;
    memcpy(s->data, p, (size_t)n);
    s->data[n] = 0;
    return s;
}

void jw_str_retain(WStr *s) {
    if (s->rc > 0) s->rc++;
}

void jw_str_release(WStr *s) {
    if (s->rc > 0 && --s->rc == 0) bytes_free(s, str_bytes(s->len));
}

WStr *jw_str_concat(WStr *a, WStr *b);

// `s = s + b`: s の持ち主が 1 人ならその場で後ろに足す。slot は変数 s の場所。
// 戻り値は新しい参照で、このあと s の古い値（*slot）が drop される。その場で足したときは
// （取り直して場所が変わっていれば *slot も新しい場所にして）数を 1 増やして返す
WStr *jw_str_append(WStr **slot, WStr *b) {
    WStr *a = *slot;
    if (a->rc != 1) return jw_str_concat(a, b);
    int64_t n = a->len + b->len;
    size_t old = str_bytes(a->len), need = str_bytes(n);
    if (need != old) {
        if (((old + 15) >> 4) > SCLASS) {
            a = jw_checked_realloc(a, need);
            if (!a) jw_oom();
        } else {
            WStr *s = bytes_alloc(need);
            memcpy(s, a, sizeof(WStr) + (size_t)a->len);
            bytes_free(a, old);
            a = s;
        }
    }
    memcpy(a->data + a->len, b->data, (size_t)b->len);
    a->len = n;
    a->data[n] = 0;
    a->rc = 2;
    *slot = a;
    return a;
}

WStr *jw_str_concat(WStr *a, WStr *b) {
    WStr *s = str_alloc(a->len + b->len);
    memcpy(s->data, a->data, (size_t)a->len);
    memcpy(s->data + a->len, b->data, (size_t)b->len);
    return s;
}

// バイトの辞書順（Rust の str の比較と同じ）
int32_t jw_str_cmp(WStr *a, WStr *b) {
    int64_t n = a->len < b->len ? a->len : b->len;
    int c = memcmp(a->data, b->data, (size_t)n);
    if (c) return c < 0 ? -1 : 1;
    return a->len < b->len ? -1 : a->len > b->len ? 1 : 0;
}

int32_t jw_str_eq(WStr *a, WStr *b) { return a->len == b->len && memcmp(a->data, b->data, (size_t)a->len) == 0; }

// ---------------------------------------------------------------- 文字にする

WSb *jw_sb_new(void) {
    WSb *b = bytes_alloc(sizeof(WSb));
    b->cap = sizeof b->inline_buf;
    b->len = 0;
    b->p = b->inline_buf;
    return b;
}

static void sb_drop(WSb *b) {
    if (b->p != b->inline_buf) bytes_free(b->p, (size_t)b->cap);
    bytes_free(b, sizeof(WSb));
}

void jw_sb_push(WSb *b, const char *p, int64_t n) {
    if (b->len + n > b->cap) {
        int64_t cap = b->cap;
        while (b->len + n > cap) cap *= 2;
        char *np = bytes_alloc((size_t)cap);
        memcpy(np, b->p, (size_t)b->len);
        if (b->p != b->inline_buf) bytes_free(b->p, (size_t)b->cap);
        b->p = np;
        b->cap = cap;
    }
    memcpy(b->p + b->len, p, (size_t)n);
    b->len += n;
}

void jw_sb_cstr(WSb *b, const char *s) { jw_sb_push(b, s, (int64_t)strlen(s)); }
void jw_sb_str(WSb *b, WStr *s) { jw_sb_push(b, s->data, s->len); }

void jw_sb_int(WSb *b, int64_t v, int32_t is_unsigned) {
    char buf[32];
    int n = fmt_int(buf, sizeof buf, v, is_unsigned);
    jw_sb_push(b, buf, n);
}

// 整数を文字列にする（`"{n}"` の n だけのとき）
WStr *jw_str_int(int64_t v, int32_t is_unsigned) {
    char buf[32];
    int n = fmt_int(buf, sizeof buf, v, is_unsigned);
    return jw_str_new(buf, n);
}

// Rust の `{:?}` と同じ形（いちばん短く元に戻る桁。1e16 以上と 1e-4 未満は指数の形）
static void fmt_f64(char *out, double x) {
    if (isnan(x)) {
        strcpy(out, "NaN");
        return;
    }
    if (isinf(x)) {
        strcpy(out, x < 0 ? "-inf" : "inf");
        return;
    }
    if (x == 0) {
        strcpy(out, signbit(x) ? "-0.0" : "0.0");
        return;
    }
    char tmp[64];
    int p;
    for (p = 1; p <= 17; p++) {
        snprintf(tmp, sizeof tmp, "%.*e", p - 1, x);
        if (strtod(tmp, NULL) == x) break;
    }
    // ちょうど真ん中（printf は偶数へ丸める）なら、Rust と同じく大きいほうを選ぶ
    {
        static char exact[1200];
        snprintf(exact, sizeof exact, "%.*e", p - 1 + 800, x);
        const char *q = exact + (exact[0] == '-');
        int seen = 0, tie = 1;
        const char *r = q;
        for (; *r && *r != 'e'; r++) {
            if (*r < '0' || *r > '9') continue;
            if (seen == p) tie = *r == '5';
            else if (seen > p && *r != '0') tie = 0;
            seen++;
        }
        if (tie && seen > p) {
            // 切り捨てた桁と、丸めた結果の最後の桁が同じなら、下へ丸められている
            char up[64];
            strcpy(up, tmp);
            char *e = strchr(up, 'e');
            char *last = e - 1;
            const char *q2 = q;
            int k = 0;
            char trunc_last = 0;
            for (; *q2 && *q2 != 'e'; q2++) {
                if (*q2 < '0' || *q2 > '9') continue;
                if (k == p - 1) trunc_last = *q2;
                k++;
            }
            if (*last == trunc_last) {
                char *d = last;
                for (;;) {
                    if (*d == '.') {
                        d--;
                        continue;
                    }
                    if (*d == '-' || d < up) break;
                    if (*d == '9') {
                        *d = '0';
                        if (d == up || d[-1] == '-') break;
                        d--;
                        continue;
                    }
                    (*d)++;
                    break;
                }
                if (strtod(up, NULL) == x) strcpy(tmp, up);
            }
        }
    }
    // tmp は [-]d.ddde[+-]xx
    char digits[32];
    int nd = 0;
    const char *q = tmp;
    int neg = 0;
    if (*q == '-') {
        neg = 1;
        q++;
    }
    for (; *q && *q != 'e'; q++)
        if (*q >= '0' && *q <= '9') digits[nd++] = *q;
    int e = atoi(q + 1);
    while (nd > 1 && digits[nd - 1] == '0') nd--;
    char *o = out;
    if (neg) *o++ = '-';
    double a = fabs(x);
    if (a < 1e16 && a >= 1e-4) {
        if (e < 0) {
            *o++ = '0';
            *o++ = '.';
            for (int i = 0; i < -e - 1; i++) *o++ = '0';
            for (int i = 0; i < nd; i++) *o++ = digits[i];
        } else {
            for (int i = 0; i <= e; i++) *o++ = i < nd ? digits[i] : '0';
            *o++ = '.';
            if (nd > e + 1) {
                for (int i = e + 1; i < nd; i++) *o++ = digits[i];
            } else {
                *o++ = '0';
            }
        }
        *o = 0;
    } else {
        *o++ = digits[0];
        if (nd > 1) {
            *o++ = '.';
            for (int i = 1; i < nd; i++) *o++ = digits[i];
        }
        sprintf(o, "e%d", e);
    }
}

void jw_sb_float(WSb *b, double x) {
    char buf[64];
    fmt_f64(buf, x);
    jw_sb_cstr(b, buf);
}

WStr *jw_sb_finish(WSb *b) {
    WStr *s = jw_str_new(b->p, b->len);
    sb_drop(b);
    return s;
}

// ---------------------------------------------------------------- 数

// 小数 → 整数（Rust の `trunc().clamp(i128) as i128` を型の幅で一周させたもの）
int64_t jw_f2i(double x, int32_t bits, int32_t is_signed) {
    uint64_t u;
    if (isnan(x)) return 0;
    double t = trunc(x);
    if (t >= 0x1p127) {
        u = ~(uint64_t)0;
    } else if (t <= -0x1p127) {
        u = 0;
    } else if (t > -0x1p63 && t < 0x1p63) {
        u = (uint64_t)(int64_t)t;
    } else {
        double r = fmod(t, 0x1p64);
        if (r < 0) r += 0x1p64;
        u = (uint64_t)r;
    }
    if (bits < 64) {
        uint64_t m = ((uint64_t)1 << bits) - 1;
        u &= m;
        if (is_signed && (u >> (bits - 1)) & 1) u |= ~m;
    }
    return (int64_t)u;
}

// ---------------------------------------------------------------- リスト


// リストの中身は own の先と同じく、頭に世代を持つ区画に置く（要素を借りた ref はこの区画と世代を持つ）。
// 要素を借りるときは世代に「借りられた」の印（2 のビット）と「借りられたことがある」の印（LIST_SHARED）を立てる（書き出したコード）。
// - 要素が動く操作（長さが変わる・並べ替える）では、2 のビットがあるときだけ世代を進めてそれを消し、要素を借りた ref を無効にする
//   （`why` がその位置）。なければ今の世代を持つ ref はないので何もしない（push のたびに記録を書かずに済む）。
// - 一度も借りられていない中身は malloc の領域そのもの（頭 16 バイト）で、realloc で伸ばし、free で返す。
//   借りられたことがあれば、古い ref が世代を読めるよう、伸ばすときは新しい領域へ写して古いほうを区画として解放し（`why` で）、
//   捨てるときも区画として解放する（OS には返さない）。
// `why` が NULL なら作りかけのリスト（ref はない）
#define LIST_SHARED ((uint64_t)1 << 62)
#define LIST_PAD 16

static int64_t list_bytes(int64_t esize, int64_t n) { return (int64_t)round8(n * (esize ? esize : 1)); }

static char *list_new_data(int64_t bytes) {
    char *raw = jw_checked_malloc((size_t)(LIST_PAD + bytes));
    if (!raw) jw_oom();
    char *d = raw + LIST_PAD;
    gen_set(d, 0);
    return d;
}

// 要素が動いた（interp の `list_moved`）
void jw_list_moved(WList *l, const WSite *why) {
    if (!l->data || !why || !(gen_get(l->data) & 2)) return;
    remember_sent(l->data, why);
    // 借用中のlive世代は下位2bitが10なので、従来の (gen & ~3) + 4 と同じ遷移。
    // counterがbit62へはみ出して保持印と衝突する前に停止する。
    if (!gen_advance(l->data, 2)) fail_generation(why);
}

// 長さ n まで入れられるようにする。取り直したら true
static int list_grow(WList *l, int64_t esize, int64_t n, const WSite *why) {
    if (n <= l->cap) return 0;
    int64_t cap = l->cap ? l->cap : 4;
    while (cap < n) cap *= 2;
    int64_t bytes = list_bytes(esize, cap);
    if (!l->data) {
        l->data = list_new_data(bytes);
    } else if (gen_get(l->data) & LIST_SHARED) {
        char *nd = list_new_data(bytes);
        memcpy(nd, l->data, (size_t)(l->len * (esize ? esize : 1)));
        block_free(l->data, list_bytes(esize, l->cap), why);
        l->data = nd;
    } else {
        char *raw = jw_checked_realloc(l->data - LIST_PAD, (size_t)(LIST_PAD + bytes));
        if (!raw) jw_oom();
        l->data = raw + LIST_PAD;
    }
    l->cap = cap;
    return 1;
}

void jw_list_reserve(WList *l, int64_t esize, int64_t n) { list_grow(l, esize, n, NULL); }

// 終わりに 1 つ場所を足して、その場所を返す
char *jw_list_push_slot(WList *l, int64_t esize, const WSite *why) {
    if (!list_grow(l, esize, l->len + 1, why)) jw_list_moved(l, why);
    return l->data + (l->len++) * esize;
}

// i 番目に場所を空けて、その場所を返す
char *jw_list_insert_slot(WList *l, int64_t esize, int64_t i, const WSite *why) {
    if (!list_grow(l, esize, l->len + 1, why)) jw_list_moved(l, why);
    memmove(l->data + (i + 1) * esize, l->data + i * esize, (size_t)((l->len - i) * esize));
    l->len++;
    return l->data + i * esize;
}

// i 番目を out へ移して詰める
void jw_list_remove(WList *l, int64_t esize, int64_t i, void *out, const WSite *why) {
    memcpy(out, l->data + i * esize, (size_t)esize);
    memmove(l->data + i * esize, l->data + (i + 1) * esize, (size_t)((l->len - i - 1) * esize));
    l->len--;
    jw_list_moved(l, why);
}

void jw_list_reverse(WList *l, int64_t esize, const WSite *why) {
    char tmp[256];
    char *big = esize > (int64_t)sizeof tmp ? jw_checked_malloc((size_t)esize) : tmp;
    for (int64_t i = 0, j = l->len - 1; i < j; i++, j--) {
        memcpy(big, l->data + i * esize, (size_t)esize);
        memcpy(l->data + i * esize, l->data + j * esize, (size_t)esize);
        memcpy(l->data + j * esize, big, (size_t)esize);
    }
    if (big != tmp) free(big);
    jw_list_moved(l, why);
}

// 中身の区画を解放する（要素の own は書き出したコードが先に解放する）
void jw_list_free(WList *l, int64_t esize, const WSite *why) {
    if (!l->data) return;
    if (gen_get(l->data) & LIST_SHARED) block_free(l->data, list_bytes(esize, l->cap), why);
    else free(l->data - LIST_PAD);
}

// 並べ替え。less(a, b, env) で「a を b より前に置くか」を比べる。手順は src/sort.rs と同じで、同じ順に比べる
// （sort_by の関数が何回・どの順で呼ばれるかが見えるため。説明も src/sort.rs）。
// 安定（jw_sort）: run を見つけ（短ければ二分挿入ソートで min_run まで伸ばす）、run どうしは powersort の決まりで合わせる。
// 安定でない（jw_sort_unstable）: 全体が並んでいればそのまま。ほかは pdqsort に近いクイックソート（深すぎたらヒープソート）。
typedef int32_t (*WLess)(void *a, void *b, void *env);

#define SORT_SMALL 20

typedef struct {
    char *a, *s, *xb, *sw;
    int64_t es, n;
    WLess less;
    void *env;
} WSort;

static inline void s_cp(char *d, const char *src, int64_t es) {
    if (es == 8) memcpy(d, src, 8);
    else memcpy(d, src, (size_t)es);
}
#define S_AT(S, i) ((S)->a + (i) * (S)->es)
#define S_LT(S, x, y) ((S)->less((x), (y), (S)->env))

static void s_swap(WSort *S, int64_t i, int64_t j) {
    s_cp(S->sw, S_AT(S, i), S->es);
    s_cp(S_AT(S, i), S_AT(S, j), S->es);
    s_cp(S_AT(S, j), S->sw, S->es);
}

static void s_reverse(WSort *S, int64_t lo, int64_t n) {
    for (int64_t l = lo, r = lo + n - 1; l < r; l++, r--) s_swap(S, l, r);
}

// 自然な run がこれより短いときは、二分挿入ソートでこの長さまで伸ばす（32〜64）
static int64_t s_minrun(int64_t n) {
    int64_t r = 0;
    while (n >= 64) { r |= n & 1; n >>= 1; }
    return n + r;
}

static uint32_t s_depth_limit(int64_t n) {
    uint32_t b = 63;
    while (!((uint64_t)n >> b)) b--;
    return 2 * b;
}

static void s_samples(int64_t n, int64_t *out) {
    uint64_t r = (uint64_t)n * 0x9E3779B97F4A7C15ull;
    for (int k = 0; k < 9; k++) {
        r ^= r >> 29;
        r *= 0xBF58476D1CE4E5B9ull;
        r ^= r >> 32;
        out[k] = (int64_t)(r % (uint64_t)n);
    }
}

static uint32_t s_power(int64_t s1, int64_t n1, int64_t n2, int64_t n) {
    uint64_t un = (uint64_t)n, a = 2 * (uint64_t)s1 + (uint64_t)n1, b = a + (uint64_t)n1 + (uint64_t)n2;
    uint32_t p = 0;
    for (;;) {
        p++;
        int x = a >= un, y = b >= un;
        if (x != y) return p;
        if (x) { a -= un; b -= un; }
        a <<= 1;
        b <<= 1;
    }
}

static void s_insertion(WSort *S, int64_t lo, int64_t n) {
    for (int64_t i = 1; i < n; i++) {
        s_cp(S->xb, S_AT(S, lo + i), S->es);
        int64_t j = i;
        while (j > 0 && S_LT(S, S->xb, S_AT(S, lo + j - 1))) {
            s_cp(S_AT(S, lo + j), S_AT(S, lo + j - 1), S->es);
            j--;
        }
        s_cp(S_AT(S, lo + j), S->xb, S->es);
    }
}

// i から始まる run の長さ。真に減っていれば *desc
static int64_t s_run_at(WSort *S, int64_t i, int *desc) {
    int64_t n = S->n;
    *desc = 0;
    if (i + 1 >= n) return n - i;
    int64_t k = i + 2;
    if (S_LT(S, S_AT(S, i + 1), S_AT(S, i))) {
        while (k < n && S_LT(S, S_AT(S, k), S_AT(S, k - 1))) k++;
        *desc = 1;
    } else {
        while (k < n && !S_LT(S, S_AT(S, k), S_AT(S, k - 1))) k++;
    }
    return k - i;
}

static void s_merge(WSort *S, int64_t lo, int64_t mid, int64_t hi) {
    if (mid == lo || mid == hi || !S_LT(S, S_AT(S, mid), S_AT(S, mid - 1))) return;
    int64_t n1 = mid - lo, es = S->es;
    memcpy(S->s, S_AT(S, lo), (size_t)(n1 * es));
    int64_t i = 0, j = mid, k = lo;
    while (i < n1 && j < hi) {
        if (S_LT(S, S_AT(S, j), S->s + i * es)) s_cp(S_AT(S, k), S_AT(S, j++), es);
        else s_cp(S_AT(S, k), S->s + (i++) * es, es);
        k++;
    }
    if (i < n1) memcpy(S_AT(S, k), S->s + i * es, (size_t)((n1 - i) * es));
}

// lo から n 個のうち、前の done 個が並んでいるとき、残りを二分探索で挿し入れる（同じものの後ろへ入れるので安定）
static void s_binsert(WSort *S, int64_t lo, int64_t done, int64_t n) {
    for (int64_t i = done < 1 ? 1 : done; i < n; i++) {
        s_cp(S->xb, S_AT(S, lo + i), S->es);
        int64_t l = 0, r = i;
        while (l < r) {
            int64_t m = l + (r - l) / 2;
            if (S_LT(S, S->xb, S_AT(S, lo + m))) r = m;
            else l = m + 1;
        }
        memmove(S_AT(S, lo + l + 1), S_AT(S, lo + l), (size_t)((i - l) * S->es));
        s_cp(S_AT(S, lo + l), S->xb, S->es);
    }
}

static void s_stable(WSort *S) {
    int64_t n = S->n, es = S->es;
    if (n < 64) { s_binsert(S, 0, 1, n); return; }
    S->s = jw_checked_malloc((size_t)(n * es));
    int64_t minrun = s_minrun(n);
    // (始め, 長さ, 次の run との境目の深さ)
    int64_t rs[130], rl[130];
    uint32_t rp[130];
    int top = 0;
    int64_t i = 0;
    while (i < n) {
        int desc;
        int64_t len = s_run_at(S, i, &desc);
        if (desc) s_reverse(S, i, len);
        if (len < minrun) {
            int64_t f = minrun < n - i ? minrun : n - i;
            s_binsert(S, i, len, f);
            len = f;
        }
        int64_t e = i + len;
        if (top > 0) {
            uint32_t p = s_power(rs[top - 1], rl[top - 1], e - i, n);
            while (top > 1 && rp[top - 2] > p) {
                s_merge(S, rs[top - 2], rs[top - 1], rs[top - 1] + rl[top - 1]);
                rl[top - 2] += rl[top - 1];
                top--;
            }
            rp[top - 1] = p;
        }
        rs[top] = i; rl[top] = e - i; rp[top] = 0; top++;
        i = e;
    }
    while (top > 1) {
        s_merge(S, rs[top - 2], rs[top - 1], rs[top - 1] + rl[top - 1]);
        rl[top - 2] += rl[top - 1];
        top--;
    }
    free(S->s);
}

// 3 つの場所のうち、値が中央のものの場所
static int64_t s_m3_at(WSort *S, int64_t i, int64_t j, int64_t k) {
    if (S_LT(S, S_AT(S, j), S_AT(S, i))) { int64_t t = i; i = j; j = t; }
    if (S_LT(S, S_AT(S, k), S_AT(S, j))) {
        j = k;
        if (S_LT(S, S_AT(S, j), S_AT(S, i))) j = i;
    }
    return j;
}

static void s_sift(WSort *S, int64_t lo, int64_t root, int64_t end) {
    for (;;) {
        int64_t child = 2 * root + 1;
        if (child >= end) return;
        if (child + 1 < end && S_LT(S, S_AT(S, lo + child), S_AT(S, lo + child + 1))) child++;
        if (!S_LT(S, S_AT(S, lo + root), S_AT(S, lo + child))) return;
        s_swap(S, lo + root, lo + child);
        root = child;
    }
}

static void s_heapsort(WSort *S, int64_t lo, int64_t n) {
    for (int64_t st = n / 2 - 1; st >= 0; st--) s_sift(S, lo, st, n);
    for (int64_t end = n - 1; end >= 1; end--) {
        s_swap(S, lo, lo + end);
        s_sift(S, lo, 0, end);
    }
}

// pdqsort に近いクイックソート。pivot は範囲の先頭に置いて分け、終わったら間に置く。anc は外側の pivot（動かない場所）
static void s_pdq(WSort *S, int64_t lo, int64_t n, char *anc, uint32_t limit) {
    while (n > SORT_SMALL) {
        if (limit == 0) { s_heapsort(S, lo, n); return; }
        limit--;
        int64_t pos[9], m[3];
        s_samples(n, pos);
        for (int g = 0; g < 3; g++) m[g] = s_m3_at(S, lo + pos[3 * g], lo + pos[3 * g + 1], lo + pos[3 * g + 2]);
        int64_t mp = s_m3_at(S, m[0], m[1], m[2]);
        s_swap(S, lo, mp);
        char *p = S_AT(S, lo);
        if (anc && !S_LT(S, anc, p)) {
            // p より後ろでないものを左へ集めて片づける
            int64_t i = lo + 1, j = lo + n - 1;
            for (;;) {
                while (i <= j && !S_LT(S, p, S_AT(S, i))) i++;
                while (i <= j && S_LT(S, p, S_AT(S, j))) j--;
                if (i > j) break;
                s_swap(S, i, j);
                i++;
                j--;
            }
            int64_t k = i - lo;
            lo += k;
            n -= k;
            anc = NULL;
            continue;
        }
        int64_t i = lo + 1, j = lo + n - 1;
        for (;;) {
            while (i <= j && S_LT(S, S_AT(S, i), p)) i++;
            while (i <= j && !S_LT(S, S_AT(S, j), p)) j--;
            if (i > j) break;
            s_swap(S, i, j);
            i++;
            j--;
        }
        int64_t mid = i - 1;
        s_swap(S, lo, mid);
        s_pdq(S, lo, mid - lo, anc, limit);
        anc = S_AT(S, mid);
        n = lo + n - mid - 1;
        lo = mid + 1;
    }
    s_insertion(S, lo, n);
}

static void s_init(WSort *S, char *data, int64_t n, int64_t es, WLess less, void *env) {
    S->a = data;
    S->n = n;
    S->es = es;
    S->less = less;
    S->env = env;
    S->xb = jw_checked_malloc((size_t)(2 * es));
    S->sw = S->xb + es;
}

void jw_sort(char *data, int64_t n, int64_t es, WLess less, void *env) {
    if (n < 2) return;
    WSort S;
    s_init(&S, data, n, es, less, env);
    s_stable(&S);
    free(S.xb);
}

void jw_sort_unstable(char *data, int64_t n, int64_t es, WLess less, void *env) {
    if (n < 2) return;
    WSort S;
    s_init(&S, data, n, es, less, env);
    int desc;
    if (s_run_at(&S, 0, &desc) == n) {
        if (desc) s_reverse(&S, 0, n);
    } else {
        s_pdq(&S, 0, n, NULL, s_depth_limit(n));
    }
    free(S.xb);
}

// 整数の sort() / sort_unstable()（比べる関数は組み込みで、呼ばれ方は見えない）。同じ値どうしは見分けがつかないので、
// 結果は上の手順と同じ。並んでいればそのまま、短ければ挿入ソート、ほかは基数ソート（最小の値を引いて、要る桁だけ 11 ビットずつ）
static uint64_t r_key(const char *p, int32_t bytes, int32_t sgn) {
    uint64_t k;
    switch (bytes) {
    case 1: k = *(const uint8_t *)p; break;
    case 2: { uint16_t v; memcpy(&v, p, 2); k = v; break; }
    case 4: { uint32_t v; memcpy(&v, p, 4); k = v; break; }
    default: memcpy(&k, p, 8); break;
    }
    if (sgn) k ^= 1ull << (bytes * 8 - 1);
    return k;
}

void jw_sort_int(char *data, int64_t n, int32_t bytes, int32_t sgn) {
    if (n < 2) return;
    int64_t es = bytes, i = 1;
    uint64_t prev = r_key(data, bytes, sgn);
    int up = 1, down = 1;
    for (; i < n && (up || down); i++) {
        uint64_t k = r_key(data + i * es, bytes, sgn);
        if (k < prev) up = 0;
        if (k >= prev) down = 0;
        prev = k;
    }
    if (up) return;
    if (down) {
        for (int64_t l = 0, r = n - 1; l < r; l++, r--) {
            char t[8];
            memcpy(t, data + l * es, (size_t)es);
            memcpy(data + l * es, data + r * es, (size_t)es);
            memcpy(data + r * es, t, (size_t)es);
        }
        return;
    }
    if (n <= 64) {
        for (int64_t j = 1; j < n; j++) {
            char x[8];
            memcpy(x, data + j * es, (size_t)es);
            uint64_t kx = r_key(x, bytes, sgn);
            int64_t m = j;
            while (m > 0 && kx < r_key(data + (m - 1) * es, bytes, sgn)) {
                memcpy(data + m * es, data + (m - 1) * es, (size_t)es);
                m--;
            }
            memcpy(data + m * es, x, (size_t)es);
        }
        return;
    }
    // 8 バイトの整数は data をそのまま鍵の場所に使う（作業用の場所は n 個ぶんだけ）
    uint64_t *buf = jw_checked_malloc((size_t)n * (bytes == 8 ? 8 : 16));
    if (!buf) jw_oom();
    uint64_t *k = bytes == 8 ? (uint64_t *)data : buf + n, *t = buf;
    uint64_t mn = ~0ull, mx = 0;
    for (int64_t j = 0; j < n; j++) {
        uint64_t v = r_key(data + j * es, bytes, sgn);
        k[j] = v;
        if (v < mn) mn = v;
        if (v > mx) mx = v;
    }
    int bits = 0;
    while (bits < 64 && ((mx - mn) >> bits)) bits++;
    int64_t cnt[2048];
    for (int sh = 0; sh < bits; sh += 11) {
        memset(cnt, 0, sizeof cnt);
        for (int64_t j = 0; j < n; j++) cnt[((k[j] - mn) >> sh) & 2047]++;
        int64_t sum = 0;
        for (int b = 0; b < 2048; b++) { int64_t c = cnt[b]; cnt[b] = sum; sum += c; }
        for (int64_t j = 0; j < n; j++) t[cnt[((k[j] - mn) >> sh) & 2047]++] = k[j];
        uint64_t *u = k; k = t; t = u;
    }
    for (int64_t j = 0; j < n; j++) {
        uint64_t v = k[j];
        if (sgn) v ^= 1ull << (bytes * 8 - 1);
        memcpy(data + j * es, &v, (size_t)es);
    }
    free(buf);
}

// ---------------------------------------------------------------- 辞書（入れた順を保つ。キーは 8 バイト: 整数・真偽値・文字列のポインタ）

typedef struct WMap {
    int64_t len, cap, esize, icap;
    char *e;
    int64_t *idx; // 位置 + 1（0 は空き）
    int32_t kind; // 0 整数・1 文字列・2 真偽値
} WMap;

static uint64_t key_hash(WMap *m, int64_t k) {
    if (m->kind == 1) {
        WStr *s = (WStr *)(intptr_t)k;
        uint64_t h = 1469598103934665603ULL;
        for (int64_t i = 0; i < s->len; i++) h = (h ^ (unsigned char)s->data[i]) * 1099511628211ULL;
        return h;
    }
    uint64_t z = (uint64_t)k + 0x9e3779b97f4a7c15ULL;
    z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
    z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
    return z ^ (z >> 31);
}

static int key_eq(WMap *m, int64_t a, int64_t b) {
    if (m->kind == 1) return jw_str_eq((WStr *)(intptr_t)a, (WStr *)(intptr_t)b);
    return a == b;
}

static int64_t map_key(WMap *m, int64_t i) { return *(int64_t *)(m->e + i * m->esize); }

static void index_put(WMap *m, int64_t i) {
    uint64_t mask = (uint64_t)m->icap - 1;
    uint64_t h = key_hash(m, map_key(m, i)) & mask;
    while (m->idx[h]) h = (h + 1) & mask;
    m->idx[h] = i + 1;
}

static void reindex(WMap *m) {
    int64_t cap = 8;
    while (cap < m->len * 2 + 2) cap *= 2;
    free(m->idx);
    m->idx = jw_checked_calloc((size_t)cap, sizeof(int64_t));
    m->icap = cap;
    for (int64_t i = 0; i < m->len; i++) index_put(m, i);
}

WMap *jw_map_new(int64_t vsize, int32_t kind) {
    WMap *m = jw_checked_calloc(1, sizeof(WMap));
    m->esize = 8 + ((vsize + 7) & ~(int64_t)7);
    m->kind = kind;
    return m;
}

int64_t jw_map_find(WMap *m, int64_t key) {
    if (!m->icap) return -1;
    uint64_t mask = (uint64_t)m->icap - 1;
    uint64_t h = key_hash(m, key) & mask;
    while (m->idx[h]) {
        int64_t i = m->idx[h] - 1;
        if (key_eq(m, map_key(m, i), key)) return i;
        h = (h + 1) & mask;
    }
    return -1;
}

// キーを終わりに足して、その位置を返す（値の場所は書き出したコードが埋める）
int64_t jw_map_add(WMap *m, int64_t key) {
    if (m->len == m->cap) {
        m->cap = m->cap ? m->cap * 2 : 4;
        m->e = jw_checked_realloc(m->e, (size_t)(m->cap * m->esize));
    }
    int64_t i = m->len++;
    *(int64_t *)(m->e + i * m->esize) = key;
    if (m->len * 2 + 2 > m->icap) reindex(m);
    else index_put(m, i);
    return i;
}

char *jw_map_val(WMap *m, int64_t i) { return m->e + i * m->esize + 8; }
int64_t jw_map_key(WMap *m, int64_t i) { return map_key(m, i); }
int64_t jw_map_len(WMap *m) { return m->len; }

// i 番目を取り除いて後ろを詰める（キーと値の解放は書き出したコードがする）
void jw_map_remove_at(WMap *m, int64_t i) {
    memmove(m->e + i * m->esize, m->e + (i + 1) * m->esize, (size_t)((m->len - i - 1) * m->esize));
    m->len--;
    reindex(m);
}

void jw_map_free(WMap *m) {
    free(m->e);
    free(m->idx);
    free(m);
}

// ---------------------------------------------------------------- 文字列のメソッド（位置と長さは文字単位）

static int is_cont(unsigned char c) { return (c & 0xC0) == 0x80; }

int64_t jw_str_chars(WStr *s) {
    int64_t n = 0;
    for (int64_t i = 0; i < s->len; i++) n += !is_cont((unsigned char)s->data[i]);
    return n;
}

static void push_str(WList *out, WStr *x) { *(WStr **)jw_list_push_slot(out, 8, NULL) = x; }

void jw_str_chars_list(WStr *s, WList *out) {
    int64_t i = 0;
    while (i < s->len) {
        int64_t j = i + 1;
        while (j < s->len && is_cont((unsigned char)s->data[j])) j++;
        push_str(out, jw_str_new(s->data + i, j - i));
        i = j;
    }
}

// `a.join(sep)`: 文字列のリストを sep でつなぐ
WStr *jw_list_join(WList *l, WStr *sep) {
    WStr **items = (WStr **)l->data;
    int64_t n = 0;
    for (int64_t i = 0; i < l->len; i++) n += items[i]->len + (i > 0 ? sep->len : 0);
    WStr *s = str_alloc(n);
    char *p = s->data;
    for (int64_t i = 0; i < l->len; i++) {
        if (i > 0) {
            memcpy(p, sep->data, (size_t)sep->len);
            p += sep->len;
        }
        memcpy(p, items[i]->data, (size_t)items[i]->len);
        p += items[i]->len;
    }
    s->data[n] = 0;
    return s;
}

// バイトのリスト（u8[]）
void jw_str_bytes(WStr *s, WList *out) {
    if (s->len == 0) return;
    jw_list_reserve(out, 1, s->len);
    memcpy(out->data, s->data, (size_t)s->len);
    out->len = s->len;
}

// 1 文字目の番号（文字列は正しい UTF-8）
int64_t jw_str_code(WStr *s, const WSite *at) {
    if (s->len == 0) {
        begin_error("空の文字列には文字がありません");
        finish_error(at, "code() は 1 文字目の番号を返します", NULL);
    }
    const unsigned char *p = (const unsigned char *)s->data;
    if (p[0] < 0x80) return p[0];
    if (p[0] < 0xE0) return ((int64_t)(p[0] & 0x1F) << 6) | (p[1] & 0x3F);
    if (p[0] < 0xF0) return ((int64_t)(p[0] & 0x0F) << 12) | ((int64_t)(p[1] & 0x3F) << 6) | (p[2] & 0x3F);
    return ((int64_t)(p[0] & 0x07) << 18) | ((int64_t)(p[1] & 0x3F) << 12) | ((int64_t)(p[2] & 0x3F) << 6) | (p[3] & 0x3F);
}

// chr(n): 番号 n の文字 1 つ
WStr *jw_chr(int64_t n, const WSite *at) {
    if (n < 0 || n > 0x10FFFF || (n >= 0xD800 && n <= 0xDFFF)) {
        char label[128];
        snprintf(label, sizeof label, "chr(%lld) は文字になりません", (long long)n);
        begin_error("文字の番号ではありません");
        finish_error(at, label, "使えるのは 0〜1114111 で、55296〜57343（サロゲート）を除く番号です");
    }
    char b[4];
    int k;
    if (n < 0x80) { b[0] = (char)n; k = 1; }
    else if (n < 0x800) { b[0] = (char)(0xC0 | (n >> 6)); b[1] = (char)(0x80 | (n & 0x3F)); k = 2; }
    else if (n < 0x10000) { b[0] = (char)(0xE0 | (n >> 12)); b[1] = (char)(0x80 | ((n >> 6) & 0x3F)); b[2] = (char)(0x80 | (n & 0x3F)); k = 3; }
    else { b[0] = (char)(0xF0 | (n >> 18)); b[1] = (char)(0x80 | ((n >> 12) & 0x3F)); b[2] = (char)(0x80 | ((n >> 6) & 0x3F)); b[3] = (char)(0x80 | (n & 0x3F)); k = 4; }
    return jw_str_new(b, k);
}

// Rust の `lines()`: \n で分け、行末の \r を除く。最後の \n のあとの空の行は数えない
void jw_str_lines(WStr *s, WList *out) {
    int64_t i = 0;
    while (i < s->len) {
        int64_t j = i;
        while (j < s->len && s->data[j] != '\n') j++;
        int64_t end = j;
        if (end > i && s->data[end - 1] == '\r') end--;
        push_str(out, jw_str_new(s->data + i, end - i));
        i = j + 1;
    }
}

static int64_t find_from(WStr *s, WStr *pat, int64_t from) {
    if (pat->len == 0) return from;
    for (int64_t i = from; i + pat->len <= s->len; i++)
        if (memcmp(s->data + i, pat->data, (size_t)pat->len) == 0) return i;
    return -1;
}

void jw_str_split(WStr *s, WStr *sep, WList *out) {
    int64_t i = 0;
    for (;;) {
        int64_t j = find_from(s, sep, i);
        if (j < 0) break;
        push_str(out, jw_str_new(s->data + i, j - i));
        i = j + sep->len;
    }
    push_str(out, jw_str_new(s->data + i, s->len - i));
}

// UTF-8 の 1 文字を読む（*len に長さ）
static uint32_t decode(const unsigned char *p, int64_t n, int64_t *len) {
    if (p[0] < 0x80 || n < 2) {
        *len = 1;
        return p[0];
    }
    if ((p[0] & 0xE0) == 0xC0) {
        *len = 2;
        return ((uint32_t)(p[0] & 0x1F) << 6) | (p[1] & 0x3F);
    }
    if ((p[0] & 0xF0) == 0xE0 && n >= 3) {
        *len = 3;
        return ((uint32_t)(p[0] & 0x0F) << 12) | ((uint32_t)(p[1] & 0x3F) << 6) | (p[2] & 0x3F);
    }
    if (n >= 4) {
        *len = 4;
        return ((uint32_t)(p[0] & 0x07) << 18) | ((uint32_t)(p[1] & 0x3F) << 12) | ((uint32_t)(p[2] & 0x3F) << 6) | (p[3] & 0x3F);
    }
    *len = 1;
    return p[0];
}

// Unicode の White_Space（Rust の `trim` と同じ）
static int is_space(uint32_t c) {
    return (c >= 9 && c <= 13) || c == 32 || c == 0x85 || c == 0xA0 || c == 0x1680 || (c >= 0x2000 && c <= 0x200A) ||
           c == 0x2028 || c == 0x2029 || c == 0x202F || c == 0x205F || c == 0x3000;
}

static void trim_range(WStr *s, int64_t *a, int64_t *b) {
    const unsigned char *p = (const unsigned char *)s->data;
    int64_t i = 0, j = s->len;
    while (i < j) {
        int64_t l;
        uint32_t c = decode(p + i, j - i, &l);
        if (!is_space(c)) break;
        i += l;
    }
    while (j > i) {
        int64_t k = j - 1;
        while (k > i && is_cont(p[k])) k--;
        int64_t l;
        uint32_t c = decode(p + k, j - k, &l);
        if (!is_space(c)) break;
        j = k;
    }
    *a = i;
    *b = j;
}

WStr *jw_str_trim(WStr *s) {
    int64_t a, b;
    trim_range(s, &a, &b);
    return jw_str_new(s->data + a, b - a);
}

WStr *jw_str_upper(WStr *s) {
    WStr *r = jw_str_new(s->data, s->len);
    for (int64_t i = 0; i < r->len; i++)
        if (r->data[i] >= 'a' && r->data[i] <= 'z') r->data[i] -= 32;
    return r;
}

WStr *jw_str_lower(WStr *s) {
    WStr *r = jw_str_new(s->data, s->len);
    for (int64_t i = 0; i < r->len; i++)
        if (r->data[i] >= 'A' && r->data[i] <= 'Z') r->data[i] += 32;
    return r;
}

int32_t jw_str_contains(WStr *s, WStr *p) { return find_from(s, p, 0) >= 0; }
int32_t jw_str_starts(WStr *s, WStr *p) { return p->len <= s->len && memcmp(s->data, p->data, (size_t)p->len) == 0; }
int32_t jw_str_ends(WStr *s, WStr *p) {
    return p->len <= s->len && memcmp(s->data + s->len - p->len, p->data, (size_t)p->len) == 0;
}

// 見つかった位置（文字単位）。なければ -1
int64_t jw_str_find(WStr *s, WStr *p) {
    int64_t b = find_from(s, p, 0);
    if (b < 0) return -1;
    int64_t n = 0;
    for (int64_t i = 0; i < b; i++) n += !is_cont((unsigned char)s->data[i]);
    return n;
}

// 文字の位置 → バイトの位置
static int64_t byte_at(WStr *s, int64_t ch) {
    int64_t i = 0, n = 0;
    while (i < s->len) {
        if (!is_cont((unsigned char)s->data[i])) {
            if (n == ch) return i;
            n++;
        }
        i++;
    }
    return s->len;
}

WStr *jw_str_slice(WStr *s, int64_t a, int64_t b, const WSite *at) {
    int64_t n = jw_str_chars(s);
    if (a < 0 || b < a || b > n) {
        char label[256];
        snprintf(label, sizeof label, "slice(%lld, %lld) ですが、長さは %lld です", (long long)a, (long long)b, (long long)n);
        begin_error("文字列の範囲の外です");
        finish_error(at, label, "0 <= 始め <= 終わり <= 長さ にしてください");
    }
    int64_t x = byte_at(s, a), y = byte_at(s, b);
    return jw_str_new(s->data + x, y - x);
}

WStr *jw_str_replace(WStr *s, WStr *from, WStr *to) {
    WSb *b = jw_sb_new();
    if (from->len == 0) {
        // Rust と同じく、文字の境目すべてに入れる
        int64_t i = 0;
        jw_sb_str(b, to);
        while (i < s->len) {
            int64_t j = i + 1;
            while (j < s->len && is_cont((unsigned char)s->data[j])) j++;
            jw_sb_push(b, s->data + i, j - i);
            jw_sb_str(b, to);
            i = j;
        }
        return jw_sb_finish(b);
    }
    int64_t i = 0;
    for (;;) {
        int64_t j = find_from(s, from, i);
        if (j < 0) break;
        jw_sb_push(b, s->data + i, j - i);
        jw_sb_str(b, to);
        i = j + from->len;
    }
    jw_sb_push(b, s->data + i, s->len - i);
    return jw_sb_finish(b);
}

WStr *jw_str_repeat(WStr *s, int64_t k, const WSite *at) {
    if (k < 0) {
        char label[64];
        snprintf(label, sizeof label, "repeat(%lld)", (long long)k);
        begin_error("負の回数はくり返せません");
        finish_error(at, label, NULL);
    }
    if (s->len == 0 || k == 0) return jw_str_new("", 0);
    // 長さ × 回数が上限を超えるなら、確保する前にエラー（interp の str_method と同じ）
    if (k > STR_MAX / s->len) {
        char label[64];
        snprintf(label, sizeof label, "repeat(%lld)", (long long)k);
        begin_error("くり返した文字列が長すぎます");
        finish_error(at, label, "文字列は 2^62 バイトまで");
    }
    WStr *r = str_alloc(s->len * k);
    for (int64_t i = 0; i < k; i++) memcpy(r->data + i * s->len, s->data, (size_t)s->len);
    return r;
}

// Rust の `trim().parse::<i64>()`
// Rust の `str::parse::<f64>`（前後の空白は除く）: [+-] のあとに inf / infinity / nan（大文字小文字は問わない）か、
// 数字 [. 数字] / . 数字 に [eE][+-]数字 が続いてよい。合っていれば strtod で読む
int32_t jw_str_to_float(WStr *s, double *out) {
    int64_t a, b;
    trim_range(s, &a, &b);
    const char *p = s->data + a;
    int64_t n = b - a, i = 0;
    if (n == 0) return 0;
    if (p[0] == '+' || p[0] == '-') i++;
    const char *word = p + i;
    int64_t wl = n - i;
    static const char *names[] = {"inf", "infinity", "nan"};
    for (int k = 0; k < 3; k++) {
        int64_t len = (int64_t)strlen(names[k]);
        if (wl != len) continue;
        int same = 1;
        for (int64_t j = 0; j < len; j++)
            if ((word[j] | 0x20) != names[k][j]) same = 0;
        if (same) {
            double v = k == 2 ? NAN : INFINITY;
            *out = p[0] == '-' ? -v : v;
            return 1;
        }
    }
    int64_t digits = 0;
    while (i < n && p[i] >= '0' && p[i] <= '9') i++, digits++;
    if (i < n && p[i] == '.') {
        i++;
        while (i < n && p[i] >= '0' && p[i] <= '9') i++, digits++;
    }
    if (digits == 0) return 0;
    if (i < n && (p[i] == 'e' || p[i] == 'E')) {
        i++;
        if (i < n && (p[i] == '+' || p[i] == '-')) i++;
        int64_t ed = 0;
        while (i < n && p[i] >= '0' && p[i] <= '9') i++, ed++;
        if (ed == 0) return 0;
    }
    if (i != n) return 0;
    char buf[128];
    char *heap = NULL, *t = buf;
    if (n + 1 > (int64_t)sizeof buf) t = heap = jw_checked_malloc((size_t)n + 1);
    memcpy(t, p, (size_t)n);
    t[n] = 0;
    *out = strtod(t, NULL);
    free(heap);
    return 1;
}

int32_t jw_str_to_int(WStr *s, int64_t *out) {
    int64_t a, b;
    trim_range(s, &a, &b);
    const char *p = s->data + a;
    int64_t n = b - a;
    int neg = 0;
    if (n > 0 && (p[0] == '+' || p[0] == '-')) {
        neg = p[0] == '-';
        p++;
        n--;
    }
    if (n == 0) return 0;
    uint64_t v = 0;
    for (int64_t i = 0; i < n; i++) {
        if (p[i] < '0' || p[i] > '9') return 0;
        uint64_t d = (uint64_t)(p[i] - '0');
        if (v > (UINT64_MAX - d) / 10) return 0;
        v = v * 10 + d;
    }
    if (neg) {
        if (v > (uint64_t)INT64_MAX + 1) return 0;
        *out = (int64_t)(0 - v);
    } else {
        if (v > (uint64_t)INT64_MAX) return 0;
        *out = (int64_t)v;
    }
    return 1;
}

// order の順に要素を並べ直す（sort_by）
void jw_permute(char *data, int64_t n, int64_t es, const int64_t *order) {
    if (n < 2) return;
    char *tmp = jw_checked_malloc((size_t)(n * es));
    for (int64_t k = 0; k < n; k++) memcpy(tmp + k * es, data + order[k] * es, (size_t)es);
    memcpy(data, tmp, (size_t)(n * es));
    free(tmp);
}

// ---------------------------------------------------------------- 関数の値（取り込んだ値の入れ物）
//
// 頭 24 バイト: 数・表（解放と写しの関数）・印（1: ムーブする・2: own を持つ・4: 元の値がムーブするものだった）。
// そのあとに取り込んだ値が並ぶ。取り込んだ値は書き換えないので、複製は入れ物を共有して数を増やす。

typedef struct WEnvVt {
    void (*drop)(void *env, const WSite *site);
    void *(*view)(void *env, const WSite *site);
} WEnvVt;

typedef struct WEnv {
    int64_t rc;
    const WEnvVt *vt;
    int64_t flags;
} WEnv;

void *jw_env_new(int64_t size, const WEnvVt *vt) {
    WEnv *e = jw_checked_calloc(1, (size_t)size);
    e->rc = 1;
    e->vt = vt;
    return e;
}

void jw_env_retain(WEnv *e) { e->rc++; }

void jw_env_release(WEnv *e, const WSite *site) {
    if (--e->rc == 0) {
        e->vt->drop(e, site);
        free(e);
    }
}

// ---------------------------------------------------------------- スレッド（spawn / task / chan）
//
// 待つ操作（recv・join・main の終わり）は、ぜんぶ 1 つの錠 g_lock と条件変数 g_cv で待つ（interp の threads.rs と同じ）。
// 状態が変わるたびに epoch を進めて blocked を 0 にするので、blocked >= alive がデッドロック。
// デッドロックは main が自分の待っている位置で知らせる（どのスレッドが見つけても同じ表示になるように）。

static WLock g_lock = WLOCK_INIT;
static WCond g_cv = WCOND_INIT;
static int64_t g_alive = 1, g_blocked, g_deadlock, g_waiters;
static uint64_t g_epoch;
static _Thread_local int g_is_worker;
// 終わったスレッドの、生きている区画の数（区画は別のスレッドで解放されうるので、合計で見る）
static int64_t g_live_done;

static void changed(void) {
    g_epoch++;
    g_blocked = 0;
    // 待っているスレッドがなければ起こさない（送るたびに起こすと遅い）
    if (g_waiters) cond_broadcast(&g_cv);
}

static _Noreturn void fail_deadlock(const WSite *at) {
    begin_error("すべてのスレッドが待ちのまま止まりました (deadlock)");
    finish_error(at, "ここで待っているが、値を送ったり終わったりできるスレッドがもうない",
                 "チャンネルの close し忘れや、join・recv の順番を確かめてください");
}

// g_lock を持って呼ぶ。ready(arg) が真になるまで待つ
static void wait_until(int (*ready)(void *), void *arg, const WSite *at) {
    for (;;) {
        if (!g_is_worker && g_deadlock) {
            unlock(&g_lock);
            fail_deadlock(at);
        }
        if (ready(arg)) return;
        g_blocked++;
        if (g_blocked >= g_alive) {
            g_deadlock = 1;
            changed();
            if (!g_is_worker) {
                unlock(&g_lock);
                fail_deadlock(at);
            }
        }
        uint64_t e = g_epoch;
        g_waiters++;
        cond_wait(&g_cv, &g_lock);
        g_waiters--;
        if (g_epoch == e && g_blocked > 0) g_blocked--;
    }
}

// チャンネル: 値の大きさ esize の輪の並び。中身は g_lock を持って触る。数（rc）は不可分の操作で数える
typedef void (*WDrop)(void *p, const WSite *site);
typedef struct WChan {
    int64_t rc;
    int64_t esize;
    WDrop drop;
    char *buf;
    int64_t head, len, cap;
    int closed;
} WChan;

WChan *jw_chan_new(int64_t esize, WDrop drop) {
    WChan *c = jw_checked_calloc(1, sizeof(WChan));
    c->rc = 1;
    c->esize = esize > 0 ? esize : 1;
    c->drop = drop;
    return c;
}

void jw_chan_retain(WChan *c) { __atomic_fetch_add(&c->rc, 1, __ATOMIC_RELAXED); }

void jw_chan_release(WChan *c, const WSite *site) {
    if (__atomic_sub_fetch(&c->rc, 1, __ATOMIC_ACQ_REL) > 0) return;
    // 受け取られずに残った値を捨てる
    for (int64_t i = 0; i < c->len; i++) {
        if (c->drop) c->drop(c->buf + ((c->head + i) & (c->cap - 1)) * c->esize, site);
    }
    free(c->buf);
    free(c);
}

void jw_chan_send(WChan *c, void *v, const WSite *at) {
    lock(&g_lock);
    if (c->closed) {
        unlock(&g_lock);
        begin_error("閉じたチャンネルに送りました");
        finish_error(at, "このチャンネルはもう close されている", "送り終わってから close してください");
    }
    if (c->len == c->cap) {
        int64_t cap = c->cap ? c->cap * 2 : 8; // いつも 2 のべき（位置は & で回す）
        char *nb = jw_checked_malloc((size_t)(cap * c->esize));
        for (int64_t i = 0; i < c->len; i++)
            memcpy(nb + i * c->esize, c->buf + ((c->head + i) & (c->cap - 1)) * c->esize, (size_t)c->esize);
        free(c->buf);
        c->buf = nb;
        c->cap = cap;
        c->head = 0;
    }
    memcpy(c->buf + ((c->head + c->len) & (c->cap - 1)) * c->esize, v, (size_t)c->esize);
    c->len++;
    changed();
    unlock(&g_lock);
}

static int chan_ready(void *p) {
    WChan *c = p;
    return c->len > 0 || c->closed;
}

// 値が来るまで待つ。閉じて空なら 0
int32_t jw_chan_recv(WChan *c, void *out, const WSite *at) {
    lock(&g_lock);
    wait_until(chan_ready, c, at);
    int32_t got = 0;
    if (c->len > 0) {
        memcpy(out, c->buf + c->head * c->esize, (size_t)c->esize);
        c->head = (c->head + 1) & (c->cap - 1);
        c->len--;
        got = 1;
    }
    unlock(&g_lock);
    return got;
}

void jw_chan_close(WChan *c, const WSite *at) {
    lock(&g_lock);
    if (c->closed) {
        unlock(&g_lock);
        begin_error("チャンネルを 2 回 close しました");
        finish_error(at, "このチャンネルはもう close されている", NULL);
    }
    c->closed = 1;
    changed();
    unlock(&g_lock);
}

// task: 新しいスレッドと、その戻り値の置き場。持ち主（task の値）だけが解放する。
// 読むだけの写し（for の要素）は下の 1 ビットを立てたポインタ（解放しない）
typedef void (*WThunk)(void *args, void *out);
typedef struct WTask {
    WThread thread;
    WThunk thunk;
    void *args;
    char *result;
    int64_t rsize;
    WDrop drop;
    int done, joined;
} WTask;

#define TASK(p) ((WTask *)(((uintptr_t)(p)) & ~(uintptr_t)1))

#ifdef _WIN32
static unsigned long __stdcall thread_start(void *arg) {
#else
static void *thread_start(void *arg) {
#endif
    WTask *t = arg;
    g_is_worker = 1;
    t->thunk(t->args, t->result);
    free(t->args);
    lock(&g_lock);
    t->done = 1;
    g_alive--;
    g_live_done += jw_live;
    changed();
    unlock(&g_lock);
    return 0;
}

// 同時に動かしてよいスレッドの数（main のほか。環境変数 JW_MAX_THREADS、スレッドを作れないときのテスト用）。-1 なら上限なし。g_lock の中で呼ぶ。
// --release で作った実行ファイル（ソースを持たない）は環境変数を見ない
static int64_t max_threads(void) {
    static int64_t m = -2;
    if (m == -2) {
        const char *s = jw_nfiles > 0 ? getenv("JW_MAX_THREADS") : NULL;
        char *end = NULL;
        m = s && *s ? strtoll(s, &end, 10) : -1;
        if (m < 0 || (end && *end)) m = -1;
    }
    return m;
}

// spawn: thunk(args, 戻り値の置き場) を新しいスレッドで動かす。args は malloc したもの（終わったら解放する）。
// 作れなければ at でエラー（生きているスレッドの数には入れない。錠を持ったまま作るので、新しいスレッドが先に終わって数を減らすことはない）
WTask *jw_spawn(WThunk thunk, void *args, int64_t rsize, WDrop drop, const WSite *at) {
    WTask *t = jw_checked_calloc(1, sizeof(WTask));
    t->thunk = thunk;
    t->args = args;
    t->rsize = rsize > 0 ? rsize : 1;
    t->result = jw_checked_calloc(1, (size_t)t->rsize);
    t->drop = drop;
    __atomic_store_n(&g_threaded, 1, __ATOMIC_RELAXED);
    lock(&g_lock);
    int64_t m = max_threads();
    if ((m >= 0 && g_alive - 1 >= m) || !thread_new(&t->thread, t)) {
        unlock(&g_lock);
        begin_error("スレッドを作れませんでした");
        finish_error(at, "ここで spawn した", "OS のスレッドの数か、メモリが足りません");
    }
    g_alive++;
    changed();
    unlock(&g_lock);
    return t;
}

void *jw_spawn_args(int64_t size) { return jw_checked_calloc(1, (size_t)(size > 0 ? size : 1)); }

static int task_ready(void *p) { return ((WTask *)p)->done; }

void jw_task_join(void *h, void *out, const WSite *at) {
    WTask *t = TASK(h);
    lock(&g_lock);
    if (t->joined) {
        unlock(&g_lock);
        begin_error("この task はもう join しました");
        finish_error(at, "join で受け取れるのは 1 回だけ", NULL);
    }
    t->joined = 1;
    wait_until(task_ready, t, at);
    unlock(&g_lock);
    memcpy(out, t->result, (size_t)t->rsize);
}

// task を捨てる: join していなければ終わるのを待ち、戻り値を捨てる
void jw_task_drop(void *h, const WSite *site) {
    if ((uintptr_t)h & 1) return;
    WTask *t = h;
    lock(&g_lock);
    int joined = t->joined;
    t->joined = 1;
    wait_until(task_ready, t, site);
    unlock(&g_lock);
    if (!joined && t->drop) t->drop(t->result, site);
    thread_join(t->thread);
    free(t->result);
    free(t);
}

void *jw_task_view(void *h) { return (void *)((uintptr_t)h | 1); }

static int all_done(void *p) {
    (void)p;
    return g_alive == 1;
}

// main の終わり: ほかのスレッドが終わるのを待つ
void jw_wait_all(const WSite *at) {
    if (!__atomic_load_n(&g_threaded, __ATOMIC_RELAXED)) return;
    lock(&g_lock);
    wait_until(all_done, NULL, at);
    unlock(&g_lock);
}

// ほかのスレッドへ送る区画: 世代を進める（送った側に残った ref は、使うとダングリングとして見つかる）
void jw_send_block(void *p, const WSite *why) {
    // 読むだけの写し（下の 1 ビット）は送れない（型検査と取り出しのエラーで来ないはず。来たら区画を壊す前に止める）
    if ((uintptr_t)p & 1) jw_bug("読むだけの写しをほかのスレッドへ送ろうとした");
    remember_sent(p, why);
    if (!gen_advance(p, 2)) fail_generation(why);
}

// ほかのスレッドへ送る文字列: 数を数えて共有しているので、複製する（定数はそのまま）
WStr *jw_str_unshare(WStr *s) {
    if (s->rc < 0) return s;
    WStr *c = jw_str_new(s->data, s->len);
    jw_str_release(s);
    return c;
}

// ほかのスレッドへ送る辞書: 文字列のキーを複製する（並びの位置は変わらないので、引く表はそのまま）
void jw_map_unshare_keys(WMap *m) {
    if (m->kind != 1) return;
    for (int64_t i = 0; i < m->len; i++) {
        WStr **k = (WStr **)(m->e + i * m->esize);
        *k = jw_str_unshare(*k);
    }
}

// ---------------------------------------------------------------- 入力

static int g_argc;
static char **g_argv;

// 1 行読む（終わりの改行は取る）。入力が終わっていれば 0
int32_t jw_read_line(WStr **out) {
    jw_flush();
    WSb *b = jw_sb_new();
    int c, any = 0;
    while ((c = getchar()) != EOF) {
        any = 1;
        if (c == '\n') break;
        char ch = (char)c;
        jw_sb_push(b, &ch, 1);
    }
    if (!any) {
        sb_drop(b);
        return 0;
    }
    if (b->len > 0 && b->p[b->len - 1] == '\r') b->len--;
    *out = jw_sb_finish(b);
    return 1;
}

// コマンドライン引数（プログラムの名前は除く）
void jw_args(WList *out) {
    for (int i = 1; i < g_argc; i++) push_str(out, jw_str_new(g_argv[i], (int64_t)strlen(g_argv[i])));
}

// ---------------------------------------------------------------- ファイル・コマンド・終了
// エラーの文は interp の `system` と同じにする（OS のエラーの文は使わない）

#ifdef _WIN32
#include <wchar.h>
__declspec(dllimport) int __stdcall MultiByteToWideChar(unsigned, unsigned long, const char *, int, wchar_t *, int);
FILE *_wfopen(const wchar_t *, const wchar_t *);
intptr_t _wspawnvp(int, const wchar_t *, const wchar_t *const *);

static wchar_t *to_wide(const char *s) {
    int n = MultiByteToWideChar(65001, 0, s, -1, NULL, 0);
    wchar_t *w = jw_checked_malloc(sizeof(wchar_t) * (size_t)(n > 0 ? n : 1));
    if (n <= 0) w[0] = 0;
    else MultiByteToWideChar(65001, 0, s, -1, w, n);
    return w;
}

static FILE *open_file(const char *path, const wchar_t *mode) {
    wchar_t *w = to_wide(path);
    FILE *f = _wfopen(w, mode);
    free(w);
    return f;
}
#define MODE_READ L"rb"
#define MODE_WRITE L"wb"
#else
#include <spawn.h>
#include <sys/wait.h>
extern char **environ;
#define open_file(path, mode) fopen(path, mode)
#define MODE_READ "rb"
#define MODE_WRITE "wb"
#endif

// 終わりに 0 のある写し（WStr は 0 で終わるとは限らない）
static char *c_str(WStr *s) {
    char *c = jw_checked_malloc((size_t)s->len + 1);
    memcpy(c, s->data, (size_t)s->len);
    c[s->len] = 0;
    return c;
}

static WStr *message(const char *head, WStr *s) {
    WSb *b = jw_sb_new();
    jw_sb_push(b, head, (int64_t)strlen(head));
    jw_sb_push(b, s->data, s->len);
    return jw_sb_finish(b);
}

// Rust の String::from_utf8 と同じく、正しい UTF-8 か調べる（長すぎる形・サロゲートは不正）
static int valid_utf8(const unsigned char *p, int64_t n) {
    int64_t i = 0;
    while (i < n) {
        unsigned c = p[i];
        int64_t k;
        unsigned lo = 0x80, hi = 0xBF;
        if (c < 0x80) { i++; continue; }
        else if (c >= 0xC2 && c <= 0xDF) k = 1;
        else if (c == 0xE0) { k = 2; lo = 0xA0; }
        else if (c >= 0xE1 && c <= 0xEC) k = 2;
        else if (c == 0xED) { k = 2; hi = 0x9F; }
        else if (c >= 0xEE && c <= 0xEF) k = 2;
        else if (c == 0xF0) { k = 3; lo = 0x90; }
        else if (c >= 0xF1 && c <= 0xF3) k = 3;
        else if (c == 0xF4) { k = 3; hi = 0x8F; }
        else return 0;
        if (i + k >= n) return 0;
        if (p[i + 1] < lo || p[i + 1] > hi) return 0;
        for (int64_t j = 2; j <= k; j++)
            if (p[i + j] < 0x80 || p[i + j] > 0xBF) return 0;
        i += k + 1;
    }
    return 1;
}

// read_file: 読めたら 1 で *out に中身、だめなら 0 で *out にエラーの文
int32_t jw_read_file(WStr *path, WStr **out) {
    char *cp = c_str(path);
    FILE *f = open_file(cp, MODE_READ);
    free(cp);
    if (!f) {
        *out = message("ファイルを読めません: ", path);
        return 0;
    }
    WSb *b = jw_sb_new();
    char buf[65536];
    size_t n;
    while ((n = fread(buf, 1, sizeof buf, f)) > 0) jw_sb_push(b, buf, (int64_t)n);
    int bad = ferror(f);
    fclose(f);
    if (bad) {
        sb_drop(b);
        *out = message("ファイルを読めません: ", path);
        return 0;
    }
    if (!valid_utf8((const unsigned char *)b->p, b->len)) {
        sb_drop(b);
        *out = message("ファイルが UTF-8 ではありません: ", path);
        return 0;
    }
    *out = jw_sb_finish(b);
    return 1;
}

// write_file: 書けたら 1、だめなら 0 で *err にエラーの文
int32_t jw_write_file(WStr *path, WStr *text, WStr **err) {
    char *cp = c_str(path);
    FILE *f = open_file(cp, MODE_WRITE);
    free(cp);
    int ok = f != NULL;
    if (f) {
        if (fwrite(text->data, 1, (size_t)text->len, f) != (size_t)text->len) ok = 0;
        if (fclose(f) != 0) ok = 0;
    }
    if (!ok) *err = message("ファイルに書けません: ", path);
    return ok;
}

#ifdef _WIN32
// 引数 1 つを、Windows の決まり（CommandLineToArgvW が元に戻せる形）でかっこむ
static void quote_arg(WSb *b, const char *a) {
    if (*a && !strpbrk(a, " \t\"")) {
        jw_sb_push(b, a, (int64_t)strlen(a));
        return;
    }
    jw_sb_push(b, "\"", 1);
    for (const char *p = a;; p++) {
        int64_t bs = 0;
        while (*p == '\\') { bs++; p++; }
        if (!*p) {
            for (int64_t i = 0; i < bs * 2; i++) jw_sb_push(b, "\\", 1);
            break;
        }
        if (*p == '"') {
            for (int64_t i = 0; i < bs * 2 + 1; i++) jw_sb_push(b, "\\", 1);
        } else {
            for (int64_t i = 0; i < bs; i++) jw_sb_push(b, "\\", 1);
        }
        jw_sb_push(b, p, 1);
    }
    jw_sb_push(b, "\"", 1);
}
#endif

// run_command: 動かして終わるのを待つ。動いたら 1 で *code に終了コード（シグナルで止まったら -1）、
// 起動できなければ 0 で *err にエラーの文。args（str のリスト）はここで解放する
int32_t jw_run_command(WStr *cmd, WList *args, int64_t *code, WStr **err) {
    jw_flush();
    int64_t n = args->len;
    WStr **items = (WStr **)args->data;
    char **argv = jw_checked_malloc(sizeof(char *) * (size_t)(n + 2));
    argv[0] = c_str(cmd);
    for (int64_t i = 0; i < n; i++) argv[i + 1] = c_str(items[i]);
    argv[n + 1] = NULL;
    int ok;
#ifdef _WIN32
    wchar_t **wargv = jw_checked_malloc(sizeof(wchar_t *) * (size_t)(n + 2));
    for (int64_t i = 0; i <= n; i++) {
        WSb *b = jw_sb_new();
        quote_arg(b, argv[i]);
        WStr *q = jw_sb_finish(b);
        char *c = c_str(q);
        jw_str_release(q);
        wargv[i] = to_wide(c);
        free(c);
    }
    wargv[n + 1] = NULL;
    wchar_t *wcmd = to_wide(argv[0]);
    intptr_t r = _wspawnvp(0 /* _P_WAIT */, wcmd, (const wchar_t *const *)wargv);
    ok = r != -1;
    if (ok) *code = (int64_t)(int32_t)r;
    free(wcmd);
    for (int64_t i = 0; i <= n; i++) free(wargv[i]);
    free(wargv);
#else
    pid_t pid;
    ok = posix_spawnp(&pid, argv[0], NULL, NULL, argv, environ) == 0;
    if (ok) {
        int st = 0;
        while (waitpid(pid, &st, 0) < 0) {}
        *code = WIFEXITED(st) ? WEXITSTATUS(st) : -1;
    }
#endif
    for (int64_t i = 0; i <= n; i++) free(argv[i]);
    free(argv);
    if (!ok) *err = message("コマンドを起動できません: ", cmd);
    for (int64_t i = 0; i < n; i++) jw_str_release(items[i]);
    jw_list_free(args, sizeof(WStr *), NULL);
    return ok;
}

// ---------------------------------------------------------------- 資源（std.fs の File など）
//
// 資源の値は「札」（64 ビット）: 世代 << 21 | 番号 << 1 | 写しの印。OS の番号やポインタは言語の値に出さない。
// 表の場所（番号）を使い回すときは世代を進め、古い札は種類・世代・状態で必ず見つける。
// 世代が上限に達した場所は退役させ、使い回さない（ref の区画と同じ考え方）。
// 資源はほかのスレッドへ渡せない（型検査で止める）ので、中身に触るのは持ち主のスレッドだけ。
// 表の確保と解放だけを g_lock で守り、読み書きの間は錠を持たない。場所は動かない（塊ごとに確保して公開する）。

#define RES_CHUNK 1024
#define RES_CHUNKS 1024
#define RES_GEN_MAX (((uint64_t)1 << 43) - 1)
#define RES_FILE 1
#define RES_TCP 2
#define RES_LISTENER 3

typedef struct WRes {
    uint64_t gen;
    uint32_t kind;
    // 0: 空き、1: 開いている、2: 退役（使い回さない）
    uint32_t state;
    FILE *fp;
    // ソケット（TCP）
    int64_t fd;
    // 空きの場所のつながり（0 で終わり）
    int64_t next_free;
} WRes;

static WRes *g_res[RES_CHUNKS];
static int64_t g_res_next = 1;
static int64_t g_res_free = 0;

static WRes *res_at(int64_t slot) {
    if (slot <= 0 || slot >= (int64_t)RES_CHUNK * RES_CHUNKS) return NULL;
    WRes *c = __atomic_load_n(&g_res[slot / RES_CHUNK], __ATOMIC_ACQUIRE);
    return c ? &c[slot % RES_CHUNK] : NULL;
}

// テスト用: JW_RES_FAIL=open / read / write / close / list / make_dir / remove / connect / listen / accept でその操作をわざと失敗させる（--release では効かない）
static int res_fail(const char *op) {
    const char *s = jw_nfiles > 0 ? getenv("JW_RES_FAIL") : NULL;
    return s && strcmp(s, op) == 0;
}

// 新しい場所を取って札を返す。場所が尽きたら 0
static uint64_t res_new(uint32_t kind, FILE *fp, int64_t fd) {
    lock(&g_lock);
    int64_t slot = 0;
    if (g_res_free > 0) {
        slot = g_res_free;
        g_res_free = res_at(slot)->next_free;
    } else if (g_res_next < (int64_t)RES_CHUNK * RES_CHUNKS) {
        slot = g_res_next++;
        if (!g_res[slot / RES_CHUNK]) {
            WRes *c = jw_checked_calloc(RES_CHUNK, sizeof(WRes));
            __atomic_store_n(&g_res[slot / RES_CHUNK], c, __ATOMIC_RELEASE);
        }
    }
    uint64_t tok = 0;
    if (slot > 0) {
        WRes *r = res_at(slot);
        r->kind = kind;
        r->state = 1;
        r->fp = fp;
        r->fd = fd;
        tok = (r->gen << 21) | ((uint64_t)slot << 1);
    }
    unlock(&g_lock);
    return tok;
}

// 札の指す開いた資源。古い札・違う種類なら実行時エラー（型検査を通ったプログラムでは起きない）。kind が 0 ならどの種類でもよい
static WRes *res_get(uint64_t tok, uint32_t kind, const WSite *at) {
    WRes *r = res_at((int64_t)((tok >> 1) & (((uint64_t)1 << 20) - 1)));
    if (!r || r->gen != (tok >> 21) || (kind && r->kind != kind) || r->state != 1) {
        begin_error("閉じた資源を使いました");
        finish_error(at, "この資源はもう閉じている", NULL);
    }
    return r;
}

// 場所を空ける。世代を進め、上限なら退役させる
static void res_release(WRes *r, int64_t slot) {
    lock(&g_lock);
    r->fp = NULL;
    if (r->gen >= RES_GEN_MAX) {
        r->state = 2;
    } else {
        r->gen++;
        r->state = 0;
        r->next_free = g_res_free;
        g_res_free = slot;
    }
    unlock(&g_lock);
}

static int64_t res_slot(uint64_t tok) { return (int64_t)((tok >> 1) & (((uint64_t)1 << 20) - 1)); }

// mode: 0 読む、1 書く（作る・空にする）、2 後ろに足す
int32_t jw_fs_open(WStr *path, int64_t mode, void **out, WStr **err) {
    if (memchr(path->data, 0, (size_t)path->len)) {
        *err = message("パスに NUL 文字があります: ", path);
        return 0;
    }
    char *cp = c_str(path);
#ifdef _WIN32
    const wchar_t *m = mode == 0 ? L"rb" : mode == 1 ? L"wb" : L"ab";
#else
    const char *m = mode == 0 ? "rb" : mode == 1 ? "wb" : "ab";
#endif
    FILE *f = res_fail("open") ? NULL : open_file(cp, m);
    free(cp);
    if (!f) {
        *err = message(mode == 0 ? "ファイルを開けません: " : "ファイルを書くために開けません: ", path);
        return 0;
    }
    uint64_t tok = res_new(RES_FILE, f, -1);
    if (!tok) {
        fclose(f);
        *err = message("開いている資源が多すぎます: ", path);
        return 0;
    }
    *out = (void *)(uintptr_t)tok;
    return 1;
}

static WStr *err_text(const char *s) {
    WSb *b = jw_sb_new();
    jw_sb_push(b, s, (int64_t)strlen(s));
    return jw_sb_finish(b);
}

// 最大 n バイト読む（終わりなら空のリスト）
int32_t jw_file_read(void *h, int64_t n, WList *out, WStr **err, const WSite *at) {
    WRes *r = res_get((uint64_t)(uintptr_t)h, RES_FILE, at);
    if (n < 0) {
        begin_error("読む大きさが負です");
        finish_error(at, "read(n) の n は 0 以上", NULL);
    }
    if (n == 0) return 1;
    int64_t want = n < 65536 ? n : 65536;
    jw_list_reserve(out, 1, want);
    size_t got = res_fail("read") ? 0 : fread(out->data, 1, (size_t)want, r->fp);
    if (got == 0 && (res_fail("read") || ferror(r->fp))) {
        clearerr(r->fp);
        jw_list_free(out, 1, NULL);
        *err = err_text("ファイルを読めません");
        return 0;
    }
    out->len = (int64_t)got;
    return 1;
}

// 残りをすべて読む（UTF-8 でなければ Err）
int32_t jw_file_read_text(void *h, WStr **out, WStr **err, const WSite *at) {
    WRes *r = res_get((uint64_t)(uintptr_t)h, RES_FILE, at);
    WSb *b = jw_sb_new();
    char buf[65536];
    size_t n;
    while (!res_fail("read") && (n = fread(buf, 1, sizeof buf, r->fp)) > 0) jw_sb_push(b, buf, (int64_t)n);
    if (res_fail("read") || ferror(r->fp)) {
        clearerr(r->fp);
        sb_drop(b);
        *err = err_text("ファイルを読めません");
        return 0;
    }
    if (!valid_utf8((const unsigned char *)b->p, b->len)) {
        sb_drop(b);
        *err = err_text("ファイルが UTF-8 ではありません");
        return 0;
    }
    *out = jw_sb_finish(b);
    return 1;
}

static int32_t file_write(WRes *r, const char *p, int64_t n, WStr **err) {
    if (res_fail("write") || fwrite(p, 1, (size_t)n, r->fp) != (size_t)n) {
        clearerr(r->fp);
        *err = err_text("ファイルに書けません");
        return 0;
    }
    return 1;
}

// 全部書く。リストはムーブされてきたので解放する
int32_t jw_file_write(void *h, WList *data, WStr **err, const WSite *at) {
    WRes *r = res_get((uint64_t)(uintptr_t)h, RES_FILE, at);
    int32_t ok = file_write(r, data->data, data->len, err);
    jw_list_free(data, 1, NULL);
    return ok;
}

int32_t jw_file_write_text(void *h, WStr *text, WStr **err, const WSite *at) {
    WRes *r = res_get((uint64_t)(uintptr_t)h, RES_FILE, at);
    return file_write(r, text->data, text->len, err);
}

// 閉じる（資源はムーブされてきた）。失敗しても場所は空ける（もう使えない）
static int sock_close(int64_t fd);

// 種類ごとに OS の資源を手放す。失敗したら 0（それでも手放したとみなす。同じ番号をもう一度閉じない）
static int res_free_os(WRes *r) {
    if (r->kind == RES_FILE) return fclose(r->fp) == 0;
    return sock_close(r->fd);
}

int32_t jw_res_close(void *h, WStr **err, const WSite *at) {
    uint64_t tok = (uint64_t)(uintptr_t)h;
    WRes *r = res_get(tok, 0, at);
    uint32_t kind = r->kind;
    int bad = !res_free_os(r) || res_fail("close");
    res_release(r, res_slot(tok));
    if (bad) {
        *err = err_text(kind == RES_FILE ? "ファイルを閉じられません（書いた内容が残っていないかもしれません）" : "接続を閉じられません");
        return 0;
    }
    return 1;
}

// 持ち主がいなくなった: 黙って閉じる（写しなら何もしない）。結果が要るなら close を使う
void jw_res_drop(void *h, const WSite *site) {
    uint64_t tok = (uint64_t)(uintptr_t)h;
    if (tok & 1 || tok == 0) return;
    WRes *r = res_get(tok, 0, site);
    res_free_os(r);
    res_release(r, res_slot(tok));
}

void *jw_res_view(void *h) { return (void *)((uintptr_t)h | 1); }

// ---------------------------------------------------------------- 環境変数・時計（std.env・std.time）

#ifdef _WIN32
__declspec(dllimport) int __stdcall QueryPerformanceCounter(int64_t *);
__declspec(dllimport) int __stdcall QueryPerformanceFrequency(int64_t *);
__declspec(dllimport) void __stdcall GetSystemTimeAsFileTime(uint64_t *);
__declspec(dllimport) int __stdcall MoveFileExW(const wchar_t *, const wchar_t *, unsigned long);
__declspec(dllimport) void __stdcall Sleep(unsigned long);
__declspec(dllimport) int __stdcall WideCharToMultiByte(unsigned, unsigned long, const wchar_t *, int, char *, int, const char *, int *);
wchar_t *_wgetenv(const wchar_t *);
#else
#include <time.h>
#endif

// 環境変数。あれば 1（UTF-8 でない値は、ないとみなす）
int32_t jw_env_get(WStr *name, WStr **out) {
    if (name->len == 0 || memchr(name->data, 0, (size_t)name->len) || memchr(name->data, '=', (size_t)name->len)) return 0;
    char *cp = c_str(name);
#ifdef _WIN32
    wchar_t *w = to_wide(cp);
    const wchar_t *v = _wgetenv(w);
    free(w);
    char *val = NULL;
    if (v) {
        int n = WideCharToMultiByte(65001, 0, v, -1, NULL, 0, NULL, NULL);
        val = jw_checked_malloc((size_t)(n > 0 ? n : 1));
        if (n <= 0) val[0] = 0;
        else WideCharToMultiByte(65001, 0, v, -1, val, n, NULL, NULL);
    }
#else
    const char *v = getenv(cp);
    char *val = NULL;
    if (v) {
        val = jw_checked_malloc(strlen(v) + 1);
        memcpy(val, v, strlen(v) + 1);
    }
#endif
    free(cp);
    if (!val) return 0;
    int64_t n = (int64_t)strlen(val);
    int ok = valid_utf8((const unsigned char *)val, n);
    if (ok) *out = jw_str_new(val, n);
    free(val);
    return ok;
}

// 単調な時計（ナノ秒。始まりの位置は決めない。壁時計の修正に影響されない）
int64_t jw_time_now_ns(void) {
#ifdef _WIN32
    int64_t c = 0, f = 1;
    QueryPerformanceCounter(&c);
    QueryPerformanceFrequency(&f);
    return (int64_t)((double)c * 1e9 / (double)f);
#else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000000000 + ts.tv_nsec;
#endif
}

// 壁時計（1970-01-01 UTC からのミリ秒。時刻の修正で戻ることがある）
int64_t jw_time_unix_ms(void) {
#ifdef _WIN32
    uint64_t t = 0; // FILETIME（1601 年からの 100 ナノ秒）と同じ並び
    GetSystemTimeAsFileTime(&t);
    return (int64_t)((t - 116444736000000000ULL) / 10000);
#else
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (int64_t)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
#endif
}

void jw_time_sleep_ms(int64_t ms, const WSite *at) {
    if (ms < 0) {
        begin_error("待つ時間が負です");
        finish_error(at, "sleep(ms) の ms は 0 以上", NULL);
    }
    jw_flush();
#ifdef _WIN32
    while (ms > 0) {
        unsigned long k = ms > 86400000 ? 86400000 : (unsigned long)ms;
        Sleep(k);
        ms -= k;
    }
#else
    struct timespec ts;
    ts.tv_sec = (time_t)(ms / 1000);
    ts.tv_nsec = (long)((ms % 1000) * 1000000);
    while (nanosleep(&ts, &ts) != 0) {}
#endif
}

// ---------------------------------------------------------------- TCP（std.net）
// 同期だけ。名前の解決は getaddrinfo。Windows はまだ（Winsock の初期化を含めて後で足す）

#ifdef _WIN32
static int sock_close(int64_t fd) { (void)fd; return 1; }

int32_t jw_net_connect(WStr *host, int64_t port, void **out, WStr **err) {
    (void)port; (void)out;
    *err = message("std.net はまだ Windows で使えません: ", host);
    return 0;
}

int32_t jw_net_listen(WStr *host, int64_t port, void **out, WStr **err) {
    (void)port; (void)out;
    *err = message("std.net はまだ Windows で使えません: ", host);
    return 0;
}

int32_t jw_tcp_read(void *h, int64_t n, WList *out, WStr **err, const WSite *at) {
    (void)h; (void)n; (void)out; (void)err; (void)at;
    return 0;
}

int32_t jw_tcp_write(void *h, WList *data, WStr **err, const WSite *at) {
    (void)h; (void)err; (void)at;
    jw_list_free(data, 1, NULL);
    return 0;
}

int32_t jw_tcp_write_text(void *h, WStr *text, WStr **err, const WSite *at) {
    (void)h; (void)text; (void)err; (void)at;
    return 0;
}

int32_t jw_tcp_set_timeout(void *h, int64_t ms, WStr **err, const WSite *at) {
    (void)h; (void)ms; (void)err; (void)at;
    return 0;
}

int32_t jw_listener_accept(void *h, void **out, WStr **err, const WSite *at) {
    (void)h; (void)out; (void)err; (void)at;
    return 0;
}

int64_t jw_listener_port(void *h, const WSite *at) {
    (void)h; (void)at;
    return 0;
}
#else
#include <errno.h>
#include <netdb.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

#ifndef MSG_NOSIGNAL
#define MSG_NOSIGNAL 0
#endif

static int sock_close(int64_t fd) { return close((int)fd) == 0; }

static void sock_quiet(int fd) {
#ifdef SO_NOSIGPIPE
    int one = 1;
    setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, sizeof one);
#else
    (void)fd;
#endif
}

// 名前を解決して、つなぐ（listen なら待ち受ける）。できたらソケット、だめなら -1
static int net_open(WStr *host, int64_t port, int listening) {
    if (port < 0 || port > 65535 || memchr(host->data, 0, (size_t)host->len)) return -1;
    char *hc = c_str(host);
    char ps[16];
    snprintf(ps, sizeof ps, "%d", (int)port);
    struct addrinfo hints, *res = NULL;
    memset(&hints, 0, sizeof hints);
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    if (listening) hints.ai_flags = AI_PASSIVE;
    int fd = -1;
    if (getaddrinfo(hc, ps, &hints, &res) == 0) {
        for (struct addrinfo *a = res; a && fd < 0; a = a->ai_next) {
            fd = socket(a->ai_family, a->ai_socktype, a->ai_protocol);
            if (fd < 0) continue;
            int ok;
            if (listening) {
                int one = 1;
                setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);
                ok = bind(fd, a->ai_addr, a->ai_addrlen) == 0 && listen(fd, 64) == 0;
            } else {
                ok = connect(fd, a->ai_addr, a->ai_addrlen) == 0;
            }
            if (!ok) {
                close(fd);
                fd = -1;
            }
        }
        freeaddrinfo(res);
    }
    free(hc);
    if (fd >= 0) sock_quiet(fd);
    return fd;
}

static int32_t net_start(WStr *host, int64_t port, int listening, void **out, WStr **err) {
    int fd = res_fail(listening ? "listen" : "connect") ? -1 : net_open(host, port, listening);
    if (fd < 0) {
        *err = message(listening ? "待ち受けられません: " : "接続できません: ", host);
        return 0;
    }
    uint64_t tok = res_new(listening ? RES_LISTENER : RES_TCP, NULL, fd);
    if (!tok) {
        close(fd);
        *err = message("開いている資源が多すぎます: ", host);
        return 0;
    }
    *out = (void *)(uintptr_t)tok;
    return 1;
}

int32_t jw_net_connect(WStr *host, int64_t port, void **out, WStr **err) { return net_start(host, port, 0, out, err); }

int32_t jw_net_listen(WStr *host, int64_t port, void **out, WStr **err) { return net_start(host, port, 1, out, err); }

static WStr *sock_error(const char *what) {
    return err_text(errno == EAGAIN || errno == EWOULDBLOCK ? "時間切れです" : what);
}

// 最大 n バイト受け取る（相手が閉じたら空のリスト）
int32_t jw_tcp_read(void *h, int64_t n, WList *out, WStr **err, const WSite *at) {
    WRes *r = res_get((uint64_t)(uintptr_t)h, RES_TCP, at);
    if (n < 0) {
        begin_error("読む大きさが負です");
        finish_error(at, "read(n) の n は 0 以上", NULL);
    }
    if (n == 0) return 1;
    int64_t want = n < 65536 ? n : 65536;
    jw_list_reserve(out, 1, want);
    ssize_t got;
    do {
        got = res_fail("read") ? -1 : recv((int)r->fd, out->data, (size_t)want, 0);
    } while (got < 0 && errno == EINTR && !res_fail("read"));
    if (got < 0) {
        jw_list_free(out, 1, NULL);
        *err = sock_error("受け取れません");
        return 0;
    }
    out->len = (int64_t)got;
    return 1;
}

static int32_t sock_write(WRes *r, const char *p, int64_t n, WStr **err) {
    while (n > 0) {
        ssize_t k = res_fail("write") ? -1 : send((int)r->fd, p, (size_t)n, MSG_NOSIGNAL);
        if (k < 0 && errno == EINTR && !res_fail("write")) continue;
        if (k <= 0) {
            *err = sock_error("送れません");
            return 0;
        }
        p += k;
        n -= k;
    }
    return 1;
}

int32_t jw_tcp_write(void *h, WList *data, WStr **err, const WSite *at) {
    WRes *r = res_get((uint64_t)(uintptr_t)h, RES_TCP, at);
    int32_t ok = sock_write(r, data->data, data->len, err);
    jw_list_free(data, 1, NULL);
    return ok;
}

int32_t jw_tcp_write_text(void *h, WStr *text, WStr **err, const WSite *at) {
    WRes *r = res_get((uint64_t)(uintptr_t)h, RES_TCP, at);
    return sock_write(r, text->data, text->len, err);
}

// 受け取り・送りの待ち時間（ミリ秒。0 なら待ち続ける）
int32_t jw_tcp_set_timeout(void *h, int64_t ms, WStr **err, const WSite *at) {
    WRes *r = res_get((uint64_t)(uintptr_t)h, RES_TCP, at);
    if (ms < 0) {
        begin_error("待ち時間が負です");
        finish_error(at, "set_timeout(ms) の ms は 0 以上", NULL);
    }
    struct timeval tv;
    tv.tv_sec = (time_t)(ms / 1000);
    tv.tv_usec = (suseconds_t)((ms % 1000) * 1000);
    if (setsockopt((int)r->fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv) != 0 ||
        setsockopt((int)r->fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv) != 0) {
        *err = err_text("待ち時間を決められません");
        return 0;
    }
    return 1;
}

int32_t jw_listener_accept(void *h, void **out, WStr **err, const WSite *at) {
    WRes *r = res_get((uint64_t)(uintptr_t)h, RES_LISTENER, at);
    int fd;
    do {
        fd = res_fail("accept") ? -1 : accept((int)r->fd, NULL, NULL);
    } while (fd < 0 && errno == EINTR && !res_fail("accept"));
    if (fd < 0) {
        *err = err_text("接続を受けられません");
        return 0;
    }
    sock_quiet(fd);
    uint64_t tok = res_new(RES_TCP, NULL, fd);
    if (!tok) {
        close(fd);
        *err = err_text("開いている資源が多すぎます");
        return 0;
    }
    *out = (void *)(uintptr_t)tok;
    return 1;
}

// 待ち受けている番号（listen で 0 を渡したときに OS が選んだもの）
int64_t jw_listener_port(void *h, const WSite *at) {
    WRes *r = res_get((uint64_t)(uintptr_t)h, RES_LISTENER, at);
    struct sockaddr_storage ss;
    socklen_t n = sizeof ss;
    if (getsockname((int)r->fd, (struct sockaddr *)&ss, &n) != 0) return -1;
    if (ss.ss_family == AF_INET) return ntohs(((struct sockaddr_in *)&ss)->sin_port);
    if (ss.ss_family == AF_INET6) return ntohs(((struct sockaddr_in6 *)&ss)->sin6_port);
    return -1;
}
#endif

// ---------------------------------------------------------------- ディレクトリ（std.fs）

#ifdef _WIN32
#include <io.h>
#include <direct.h>
#else
#include <dirent.h>
#include <sys/stat.h>
#endif

static int path_ok(WStr *path, WStr **err) {
    if (memchr(path->data, 0, (size_t)path->len)) {
        *err = message("パスに NUL 文字があります: ", path);
        return 0;
    }
    return 1;
}

// あるか（ファイルでもディレクトリでも）
int32_t jw_fs_exists(WStr *path) {
    if (memchr(path->data, 0, (size_t)path->len)) return 0;
    char *cp = c_str(path);
#ifdef _WIN32
    wchar_t *w = to_wide(cp);
    int ok = _waccess(w, 0) == 0;
    free(w);
#else
    struct stat st;
    int ok = stat(cp, &st) == 0;
#endif
    free(cp);
    return ok;
}

static int name_cmp(const void *a, const void *b) { return jw_str_cmp(*(WStr *const *)a, *(WStr *const *)b); }

// 中の名前のリスト（`.` と `..` は除き、名前の順に並べる）
int32_t jw_fs_list(WStr *path, WList *out, WStr **err) {
    if (!path_ok(path, err)) return 0;
    char *cp = c_str(path);
    int ok = 1;
#ifdef _WIN32
    WSb *pat = jw_sb_new();
    jw_sb_push(pat, cp, (int64_t)strlen(cp));
    jw_sb_push(pat, "\\*", 2);
    WStr *ps = jw_sb_finish(pat);
    char *pc = c_str(ps);
    jw_str_release(ps);
    wchar_t *w = to_wide(pc);
    free(pc);
    struct _wfinddata64_t fd;
    intptr_t h = _wfindfirst64(w, &fd);
    free(w);
    if (h == -1) {
        ok = 0;
    } else {
        do {
            if (wcscmp(fd.name, L".") == 0 || wcscmp(fd.name, L"..") == 0) continue;
            int n = WideCharToMultiByte(65001, 0, fd.name, -1, NULL, 0, NULL, NULL);
            char *u = jw_checked_malloc((size_t)(n > 0 ? n : 1));
            if (n <= 0) u[0] = 0;
            else WideCharToMultiByte(65001, 0, fd.name, -1, u, n, NULL, NULL);
            push_str(out, jw_str_new(u, (int64_t)strlen(u)));
            free(u);
        } while (_wfindnext64(h, &fd) == 0);
        _findclose(h);
    }
#else
    DIR *d = res_fail("list") ? NULL : opendir(cp);
    if (!d) {
        ok = 0;
    } else {
        struct dirent *e;
        while ((e = readdir(d)) != NULL) {
            if (strcmp(e->d_name, ".") == 0 || strcmp(e->d_name, "..") == 0) continue;
            int64_t n = (int64_t)strlen(e->d_name);
            // UTF-8 でない名前は str にできないので、読めないとする
            if (!valid_utf8((const unsigned char *)e->d_name, n)) { ok = 0; break; }
            push_str(out, jw_str_new(e->d_name, n));
        }
        closedir(d);
    }
#endif
    free(cp);
    if (!ok) {
        for (int64_t i = 0; i < out->len; i++) jw_str_release(((WStr **)out->data)[i]);
        if (out->data) jw_list_free(out, sizeof(WStr *), NULL);
        *err = message("ディレクトリを読めません: ", path);
        return 0;
    }
    if (out->len > 1) qsort(out->data, (size_t)out->len, sizeof(WStr *), name_cmp);
    return 1;
}

int32_t jw_fs_make_dir(WStr *path, WStr **err) {
    if (!path_ok(path, err)) return 0;
    char *cp = c_str(path);
#ifdef _WIN32
    wchar_t *w = to_wide(cp);
    int ok = !res_fail("make_dir") && _wmkdir(w) == 0;
    free(w);
#else
    int ok = !res_fail("make_dir") && mkdir(cp, 0777) == 0;
#endif
    free(cp);
    if (!ok) *err = message("ディレクトリを作れません: ", path);
    return ok;
}

// ファイルか、空のディレクトリを消す
int32_t jw_fs_remove(WStr *path, WStr **err) {
    if (!path_ok(path, err)) return 0;
    char *cp = c_str(path);
#ifdef _WIN32
    wchar_t *w = to_wide(cp);
    int ok = !res_fail("remove") && (_wremove(w) == 0 || _wrmdir(w) == 0);
    free(w);
#else
    int ok = !res_fail("remove") && remove(cp) == 0;
#endif
    free(cp);
    if (!ok) *err = message("消せません: ", path);
    return ok;
}

// 名前を変える。to があれば置き換える（同じファイルシステムの中なら、読む側には古いか新しいかのどちらかが見える）
int32_t jw_fs_rename(WStr *from, WStr *to, WStr **err) {
    if (!path_ok(from, err) || !path_ok(to, err)) return 0;
    char *cf = c_str(from);
    char *ct = c_str(to);
#ifdef _WIN32
    wchar_t *wf = to_wide(cf);
    wchar_t *wt = to_wide(ct);
    int ok = !res_fail("rename") && MoveFileExW(wf, wt, 0x1 | 0x8) != 0 /* REPLACE_EXISTING | WRITE_THROUGH */;
    free(wf);
    free(wt);
#else
    int ok = !res_fail("rename") && rename(cf, ct) == 0;
#endif
    free(cf);
    free(ct);
    if (!ok) *err = message("名前を変えられません: ", from);
    return ok;
}

void jw_exit(int64_t code) {
    jw_flush();
    quit((int)code);
}

// ---------------------------------------------------------------- 入口

void jw_main(void);

#ifdef _WIN32
// Windows: 端末の文字コードを UTF-8 にし、出力の改行を変えないようにし、引数は UTF-16 から UTF-8 に直す（windows.h なしで宣言する）
#include <wchar.h>
__declspec(dllimport) int __stdcall SetConsoleOutputCP(unsigned);
__declspec(dllimport) int __stdcall SetConsoleCP(unsigned);
__declspec(dllimport) wchar_t *__stdcall GetCommandLineW(void);
__declspec(dllimport) wchar_t **__stdcall CommandLineToArgvW(const wchar_t *, int *);
__declspec(dllimport) int __stdcall WideCharToMultiByte(unsigned, unsigned long, const wchar_t *, int, char *, int, const char *, int *);
int _setmode(int, int);

static void win_setup(int *argc, char ***argv) {
    SetConsoleOutputCP(65001);
    SetConsoleCP(65001);
    // 改行を \r\n に変えずにそのまま出す（interp・Go・Rust と同じ。0x8000 は _O_BINARY）
    _setmode(_fileno(stdout), 0x8000);
    _setmode(_fileno(stderr), 0x8000);
    int n = 0;
    wchar_t **w = CommandLineToArgvW(GetCommandLineW(), &n);
    if (!w) return;
    char **a = jw_checked_malloc(sizeof(char *) * (size_t)(n + 1));
    for (int i = 0; i < n; i++) {
        int len = WideCharToMultiByte(65001, 0, w[i], -1, NULL, 0, NULL, NULL);
        a[i] = jw_checked_malloc((size_t)len);
        WideCharToMultiByte(65001, 0, w[i], -1, a[i], len, NULL, NULL);
    }
    a[n] = NULL;
    *argc = n;
    *argv = a;
}
#endif

int main(int argc, char **argv) {
#ifdef _WIN32
    win_setup(&argc, &argv);
#endif
    g_argc = argc;
    g_argv = argv;
    jw_main();
    jw_flush();
    if (getenv("JW_LEAKS")) fprintf(stderr, "[live] %lld\n", (long long)(jw_live + g_live_done));
    if (getenv("JW_RETIREMENTS")) fprintf(stderr, "[retired] %llu\n", (unsigned long long)retired_count());
    return 0;
}
