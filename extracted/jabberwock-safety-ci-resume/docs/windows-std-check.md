# Windows で標準ライブラリを確かめる手順

std.net（Winsock）と std.tls（Schannel）は、Linux の上の mingw と wine でしか確かめていない。
本物の Windows で次の 2 つを試す。どちらも、リポジトリ直下の PowerShell から実行する。
clang の入れ方は [native.md](native.md) の「Windows」を見る。

## 1. 自動テスト（std.net・std.http）

```
cargo test --release --test jwc -- jwc_std_net jwc_std_http
```

- 2 件とも `ok` なら成功。jwc を作るところから始まるので、数分かかる。
- std.tls・std.gzip・std.pg のテストは Linux だけで動く（std.tls は試験用の発行元を Windows の信頼の置き場所に入れずには試せないため。std.gzip は gzip のコマンド、std.pg は PostgreSQL のコマンドを使うため）。
- std.http のサーバーは、Windows 向けに作ったものを wine で動かし、curl から答えられることを確かめた（2026-10-31）。本物の Windows でも server_check と jwc_std_http_server が通った（2026-10-31、まつさん）。
- std.pg の TLS（tls: true）は、Windows 向けに作ったものを wine で動かし、手元の PostgreSQL 16 に Schannel でつながることを確かめた（2026-10-31。本物の Windows ではまだ）。

## 2. 本物のサイトへの https（std.tls）

次の内容を `tls_check.jw` として保存する。場所はどこでもよい。

```
use std.http;

fn main() {
    let urls = [
        "https://example.com/",
        "https://wrong.host.badssl.com/",
        "https://expired.badssl.com/",
        "https://self-signed.badssl.com/",
    ];
    for u in urls {
        match http.get(u) {
            case Ok(r) -> print("Ok", r.status, r.url, r.body.len());
            case Err(e) -> print("Err", e);
        }
    }
}
```

jwc を作り、それで `tls_check.jw` を作って動かす。

```
cargo build --release
.\target\release\jabberwock.exe build -o jwc.exe jwc\main.jw
.\jwc.exe build -o tls_check.exe tls_check.jw
.\tls_check.exe
```

`.\jwc.exe build` には、PATH の clang が使われる。別の clang を使うときは `--clang C:\LLVM\bin\clang.exe` を足す。

想定どおりなら、出力は次の 4 行になる。

```
Ok 200 https://example.com/ （本文の長さ）
Err TLS の証明書を確かめられません（名前が合いません）: wrong.host.badssl.com
Err TLS の証明書を確かめられません（期限が切れています）: expired.badssl.com
Err TLS の証明書を確かめられません（信頼できる発行元につながりません）: self-signed.badssl.com
```

- 1 行目が Err なら、Schannel で TLS 1.2 の握手ができていない。
- 2〜4 行目のどれかが Ok なら、証明書を確かめられていない。安全の問題なので、すぐ知らせてほしい。
- 理由の文が違う場合（`0x800b....` の番号が出るなど）は、その行をそのまま知らせてほしい。
- 会社のネットワークなどで、外につなぐ前に証明書を差し替える中継（プロキシ）があると、結果が変わることがある。

## 3. 追加した標準ライブラリ（2026-10-31 の分）

fs の追加分・date・csv・regex・hash・base64・random と、HTTP サーバー・std.pg を、サンプル（`samples/windows/`）で確かめる。
どれも Linux の上の mingw と wine では想定どおりに動いた。本物の Windows でも、自動テスト 9 件・lib_check・server_check が通った（2026-10-31、まつさん。pg_check は未実行）。リポジトリ直下の PowerShell から実行する。

### 3.1 自動テスト

```
cargo test --release --test jwc -- jwc_std
```

- Windows で動く `jwc_std_*` は 9 件（fs・net・http・http_server・date_csv_regex・env_time_json・hash_random・url_fs_time・browser）。
  `test result: ok. 9 passed; 0 failed` なら成功。失敗したら、その試験の名前と、`panicked at` から後ろをそのまま知らせてほしい。

### 3.2 サンプル

jwc を作る（2 の手順と同じ。もう作ってあっても、新しい標準ライブラリを入れるために作り直す）。

```
cargo build --release
.\target\release\jabberwock.exe build -o jwc.exe jwc\main.jw
```

**lib_check**（fs・date・csv・regex・hash・base64・random）:

```
.\jwc.exe build -o lib_check.exe samples\windows\lib_check.jw
.\lib_check.exe
```

今いるフォルダーの下に `jw_lib_check` を作って、最後に消す。想定どおりなら、出力は次のとおり（2 行目の日付の行は、今の時刻によらず `date true`）。

```
fs 17 true false false true
fs true false [b.txt]
fs Err 写す元と先が同じです: jw_lib_check/a.txt
fs stat の Err あり
fs false
date 2026-10-31T10:37:52.250Z 1793443072250 6 304
date true
csv [[名前, 点], [山田, 太郎, 90]] true
regex 2026/10と2027/1 2
hash ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
base64 YWJj
random 16 true
おわり
```

**server_check**（HTTP サーバー）:

```
.\jwc.exe build -o server_check.exe samples\windows\server_check.jw
.\server_check.exe
```

- 動いたままになり、すぐに `待ち受けを始めました: http://127.0.0.1:8080/` と出る（出なければ print の書き出しの問題なので知らせてほしい）。開くたびに `受けた 1 GET /` のような行がすぐ出る。
- ブラウザで `http://127.0.0.1:8080/` を開き、「こんにちは、だれか さん」と出れば成功。名前を入れて「送る」を押すと、その名前に変わる。
- `http://127.0.0.1:8080/stop` を開くと「終わります」と出て、PowerShell に `おわり` が出て終わる。
- 別の PowerShell から `curl.exe -i http://127.0.0.1:8080/` でもよい（`curl` ではなく `curl.exe` と書く。`curl` だけだと PowerShell の別のコマンドになる）。
- `Err` と出てすぐ終わったら、8080 番をほかのプログラムが使っている。その行を知らせてほしい。
- Windows のファイアウォールの確かめが出たら、「キャンセル」でよい（127.0.0.1 だけで待ち受けるので、外からはつながらない）。

**local_time**（地域の時刻）:

```
.\jwc.exe build -o local_time.exe samples\windows\local_time.jw
.\local_time.exe
```

- 日本の設定なら、`地域` の行の終わりが `+09:00`、`時差  32400000`、`夏     2026-07-01T21:00:00+09:00`。
- `1969` の行は Windows では `Err`（Windows の localtime が 1970 年より前に答えないため）でよい。
- 環境変数 TZ を設定していると、そちらに従う（Windows の TZ は `JST-9` の形。`Asia/Tokyo` の形は正しく読まれない）。

**udp_check**（std.net の UDP）:

```
.\jwc.exe build -o udp_check.exe samples\windows\udp_check.jw
.\udp_check.exe
```

- 想定どおりなら 8 行で、`ports ok: true`・`got 'hello udp' from 127.0.0.1 same port: true`・`cut len 10 last 9`・`empty len 0`・
  `timeout err: 時間切れです`・`bad host: 送り先の名前が分かりません: no.such.host.invalid`・`rebind: UDP で待ち受けられません: 127.0.0.1`・`unbound`。
- 127.0.0.1 だけを使うので、ファイアウォールの確かめが出たら「キャンセル」でよい。

**pg_check**（std.pg。PostgreSQL のサーバーがあるときだけ）:

```
.\jwc.exe build -o pg_check.exe samples\windows\pg_check.jw
.\pg_check.exe localhost 5432 postgres （パスワード） postgres off
```

- 引数は「ホスト ポート ユーザー パスワード データベース tls」。最後を `on` にすると TLS でつなぐ（サーバーの証明書が Windows に信頼されていること）。
- 想定どおりなら `version Some(PostgreSQL …)`・`tls Some(false) echo Some(こんにちは!)`・`insert 2`・`rows [[Some(1), Some(山田)], [Some(2), None]]`・`おわり`。
  `on` なら 2 行目が `tls Some(true)`。サーバーがなければ、ここは飛ばしてよい。
