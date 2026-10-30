# std.tls の試験用の証明書

`tests/jwc.rs` の `jwc_std_tls` が `openssl s_server` に持たせる。試験のためだけのもので、秘密鍵も公開している。

- `ca.pem`: 試験用の発行元（EC P-256。期限は 2126 年）。発行元の鍵は残していない。
- `server.pem` / `server.key`: `localhost`（SAN は DNS:localhost だけ）の証明書と鍵。`ca.pem` が発行。

作り直すときは openssl で同じ形の発行元と証明書を作り、`ca.pem`・`server.pem`・`server.key` を置き換える。
