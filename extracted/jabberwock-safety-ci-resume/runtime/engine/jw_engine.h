/* jw_engine.h: ブラウザのエンジンとの境目（C の関数だけ）。
 *
 * jabberwock のブラウザ（browser.jw）はこのヘッダーの関数だけを使う。
 * エンジン（WebView2・試験用の偽エンジン・あとで Servo や jabberwock 製）は、
 * それぞれこの関数を 1 組ずつ実装する。差し替えはリンクするファイルを変えるだけ。
 *
 * 決まり:
 * - 呼び返しはしない。エンジンはイベントを溜め、呼ぶ側が jwe_poll / jwe_wait で取りに行く。
 * - すべての関数は jwe_open を呼んだスレッドから呼ぶ。
 * - 文字列はすべて UTF-8。長さつき（終わりの 0 は要らない）。
 * - イベントの中の文字列はエンジンが持ち、次に jwe_poll / jwe_wait / jwe_close を呼ぶまで有効。
 *   取っておくなら呼ぶ側で写す。
 * - タブは 1 から始まる番号。閉じた番号は使い回さない。0 は「タブではない」。
 * - アカウント・同期・プロファイル・パスワード保存の関数は置かない（作らない）。
 */
#ifndef JW_ENGINE_H
#define JW_ENGINE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define JWE_ABI_VERSION 1

typedef struct JweEngine JweEngine;

/* エンジンができること。jwe_info で返す */
enum {
    JWE_CAP_SCRIPT = 1u << 0,      /* ページの JavaScript が動く */
    JWE_CAP_CHROME_VIEW = 1u << 1, /* 外枠（アドレスバーなど）を HTML で表示できる（jwe_chrome_*） */
    JWE_CAP_PERSIST = 1u << 2,     /* Cookie などをディスクに残せる */
    JWE_CAP_FRAMELESS = 1u << 3    /* OS のタイトルバーがなく、外枠が窓の一番上まで来る（外枠が ー □ × を出し、jwe_window を呼ぶ） */
};

typedef struct {
    uint32_t abi_version; /* JWE_ABI_VERSION */
    uint32_t caps;        /* JWE_CAP_* の組み合わせ */
    const char *name;     /* 例 "webview2"。0 で終わる。エンジンが持つ */
    const char *version;  /* エンジンの版。0 で終わる。エンジンが持つ */
} JweInfo;

typedef struct {
    const char *title;    uint32_t title_len;
    const char *data_dir; uint32_t data_dir_len; /* Cookie などの置き場所（1 か所だけ） */
    uint32_t width, height;                      /* 論理ピクセル */
    uint32_t ephemeral;                          /* 1: 何もディスクに残さない（終わると全部消える） */
} JweConfig;

typedef enum {
    JWE_OK = 0,
    JWE_ERR_UNAVAILABLE = 1, /* エンジンが使えない（例: WebView2 の実行環境が入っていない） */
    JWE_ERR_BAD_TAB = 2,     /* その番号のタブはない */
    JWE_ERR_BAD_ARG = 3,
    JWE_ERR_UNSUPPORTED = 4, /* このエンジンにはその機能がない（JweInfo.caps を見る） */
    JWE_ERR_FAILED = 5
} JweStatus;

typedef enum {
    JWE_EV_NONE = 0,
    JWE_EV_READY,             /* エンジンの準備ができた。これより前の navigate も受け付けて後で動かす */
    JWE_EV_CLOSE_REQUESTED,   /* 窓の × が押された。閉じるかどうかは呼ぶ側が決める */
    JWE_EV_NAV_STARTED,       /* tab, text = URL */
    JWE_EV_NAV_FINISHED,      /* tab, text = URL, code = 0 成功 / それ以外は失敗 */
    JWE_EV_URL_CHANGED,       /* tab, text = URL（ページ内の移動・リダイレクトも） */
    JWE_EV_TITLE_CHANGED,     /* tab, text = 題 */
    JWE_EV_HISTORY_CHANGED,   /* tab, code = 1 なら戻れる | 2 なら進める */
    JWE_EV_NEW_TAB_REQUESTED, /* tab = 元のタブ, text = URL（target=_blank など）。開くかは呼ぶ側が決める */
    JWE_EV_CHROME_MESSAGE,    /* 外枠の HTML から来た文字列（text） */
    JWE_EV_KEY,               /* 呼ぶ側に渡すショートカット。code = JWE_KEY_* */
    JWE_EV_PERMISSION_DENIED, /* tab, text = 何を求められたか。いつも断る（記録用） */
    JWE_EV_ERROR              /* code = JweStatus, text = 説明 */
} JweEventKind;

enum { JWE_KEY_FOCUS_ADDRESS = 1, JWE_KEY_NEW_TAB, JWE_KEY_CLOSE_TAB, JWE_KEY_NEXT_TAB, JWE_KEY_PREV_TAB, JWE_KEY_RELOAD, JWE_KEY_BACK, JWE_KEY_FORWARD };

typedef struct {
    uint32_t kind; /* JweEventKind */
    uint32_t tab;
    int64_t code;
    const char *text; uint32_t text_len;
} JweEvent;

/* 開く・閉じる */
JweStatus jwe_open(const JweConfig *cfg, JweEngine **out);
void jwe_close(JweEngine *e); /* 開いているタブも全部閉じる。ephemeral なら data_dir の中も消す */
void jwe_info(const JweEngine *e, JweInfo *out);

/* イベント。あれば *out に入れて 1、なければ 0 */
int jwe_poll(JweEngine *e, JweEvent *out);
/* イベントが来るか timeout_ms たつまで待つ（OS のメッセージもここで処理する）。-1 は無期限 */
int jwe_wait(JweEngine *e, int32_t timeout_ms, JweEvent *out);

/* タブ */
JweStatus jwe_tab_new(JweEngine *e, uint32_t *out_tab);
JweStatus jwe_tab_close(JweEngine *e, uint32_t tab);
JweStatus jwe_tab_show(JweEngine *e, uint32_t tab); /* 見えるタブを切り替える */
JweStatus jwe_navigate(JweEngine *e, uint32_t tab, const char *url, uint32_t url_len);
JweStatus jwe_back(JweEngine *e, uint32_t tab);
JweStatus jwe_forward(JweEngine *e, uint32_t tab);
JweStatus jwe_reload(JweEngine *e, uint32_t tab);
JweStatus jwe_stop(JweEngine *e, uint32_t tab);

/* 外枠（JWE_CAP_CHROME_VIEW があるときだけ）。height は外枠の高さ（論理ピクセル）
 * jwe_chrome_post は jwe_chrome_load のあとなら、外枠の準備ができる前でも受け付ける（準備ができたら送る） */
JweStatus jwe_chrome_load(JweEngine *e, const char *html, uint32_t html_len, uint32_t height);
JweStatus jwe_chrome_post(JweEngine *e, const char *msg, uint32_t msg_len);
/* 外枠の高さを変える。JWE_CHROME_FULL なら外枠が窓全体を覆い、タブのページは隠れる（設定画面など） */
#define JWE_CHROME_FULL 0xFFFFFFFFu
JweStatus jwe_chrome_height(JweEngine *e, uint32_t height);

/* 窓 */
JweStatus jwe_set_title(JweEngine *e, const char *title, uint32_t title_len);

enum {
    JWE_WINDOW_DRAG = 1,        /* 押している間、マウスで窓を動かす（外枠の空いたところが押されたとき） */
    JWE_WINDOW_MINIMIZE = 2,
    JWE_WINDOW_TOGGLE_MAXIMIZE = 3,
    JWE_WINDOW_FOCUS = 4         /* 最小化を戻して手前に出す */
};
JweStatus jwe_window(JweEngine *e, uint32_t op);

/* ログインの欄にユーザー名とパスワードを入れる（パスワードの金庫から、利用者が押したときだけ呼ぶ）。
 * site は "https://example.com" のような出どころ（scheme://host[:port]。既定のポートは書かない）。
 * タブの今のページの出どころが site と完全に同じときだけ入れる（違えば JWE_ERR_BAD_ARG で何もしない）。
 * エンジンはパスワードを覚えない。入れる直前にもページの中でもう一度出どころを確かめる */
JweStatus jwe_fill_login(JweEngine *e, uint32_t tab, const char *site, uint32_t site_len,
                         const char *user, uint32_t user_len, const char *password, uint32_t password_len);

/* Cookie・キャッシュ・サイトのデータを全部消す */
JweStatus jwe_clear_site_data(JweEngine *e);

#ifdef __cplusplus
}
#endif

#endif
