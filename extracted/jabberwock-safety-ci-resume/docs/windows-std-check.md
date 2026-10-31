# Windows で std.net・std.http・std.tls を確かめる手順

std.net（Winsock）と std.tls（Schannel）は、Linux の上の mingw と wine でしか確かめていない。
本物の Windows で次の 2 つを試す。どちらも、リポジトリ直下の PowerShell から実行する。
clang の入れ方は [native.md](native.md) の「Windows」を見る。

## 1. 自動テスト（std.net・std.http）

```
cargo test --release --test jwc -- jwc_std_net jwc_std_http
```

- 2 件とも `ok` なら成功。jwc を作るところから始まるので、数分かかる。
- std.tls・std.gzip・std.pg のテストは Linux だけで動く（std.tls は試験用の発行元を Windows の信頼の置き場所に入れずには試せないため。std.gzip は gzip のコマンド、std.pg は PostgreSQL のコマンドを使うため）。
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
