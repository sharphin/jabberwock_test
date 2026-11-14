# 実行ファイルを作る（`jabberwock build`）

jabberwock のプログラムを LLVM に書き出し、clang で実行ファイルにする。インタプリタ（`jabberwock file.jw`）と同じ動き・同じエラー表示で、ずっと速い（fib(32) で約 600 倍）。

```
jabberwock build hello.jw              # hello（Windows では hello.exe）ができる
jabberwock build -o app hello.jw       # 名前を決める
jabberwock build --release hello.jw    # 実行時エラーは使った位置だけ出す（ソースを実行ファイルに入れない）
jabberwock --ll hello.jw               # 書き出した LLVM IR を見るだけ
```

- 最適化は `-O2`。clang は `PATH` の `clang` を使う。別のものは環境変数 `JW_CLANG` で指定する（例 `JW_CLANG=C:\LLVM\bin\clang.exe`）。
- まだ書き出せない書き方があると、理由を出して止まる（インタプリタでは動く）。
- 測定用の切り替え（bench/run.sh が使う）: `--no-elide`（世代の検査を全部残す）・`--no-gen-checks` / `--no-bounds-checks` / `--unchecked`（検査を消す。**安全でない**）・
  `--count-checks`（実行時に通った検査の数を、正常に終わるときに標準エラーへ `[checks] gen .. elided .. bounds ..` と出す。Linux・Windows。`exit` と実行時エラーは対象外）・`--stats`（書き出した検査の場所の数）。
- GitHub Actions（`.github/workflows/test.yml`）が main への push ごとに、Linux と Windows で全テストを実行ファイルとの突き合わせ（`JW_NATIVE=1`）つきで走らせる。Windows は公式の LLVM と Visual Studio（下の B）の組み合わせで確かめている（A の llvm-mingw はまだ確かめていない）。

## Windows で clang を入れる

どちらか一つでよい。**A がおすすめ**（ほかのものを入れなくてよく、フォルダを消せば元どおり）。

### A. llvm-mingw（zip を展開するだけ）

1. https://github.com/mstorsjo/llvm-mingw/releases を開く。
2. いちばん新しい版の `llvm-mingw-<日付>-ucrt-x86_64.zip` をダウンロードする（ARM の PC なら `aarch64`）。
3. 好きな場所（例 `C:\llvm-mingw`）に展開する。
4. その中の `bin` を PATH に足す: スタートメニューで「環境変数」→「環境変数を編集」→ ユーザー環境変数の `Path` → 新規 → `C:\llvm-mingw\bin`。
5. **新しく開いた** PowerShell で `clang --version` が出れば完了。

### B. 公式の LLVM ＋ Visual Studio Build Tools

公式の clang は、リンクに Microsoft のリンカーを使うので Build Tools も要る。

1. PowerShell で `winget install LLVM.LLVM`（または https://github.com/llvm/llvm-project/releases の `LLVM-<版>-win64.exe`。途中で「PATH に足す」を選ぶ）。
2. `winget install Microsoft.VisualStudio.2022.BuildTools`。開いた画面で「C++ によるデスクトップ開発」を選んで入れる。
3. 新しく開いた PowerShell で `clang --version`。

### 確かめる

```
cargo build --release
.\target\release\jabberwock.exe build examples\fib.jw
.\fib.exe
```

| 症状 | 直し方 |
|---|---|
| `clang を起動できません` | PATH に `bin` が入っていない。PowerShell を開き直す。だめなら `JW_CLANG` にフルパス |
| `unable to find a Visual Studio installation` / `link.exe` が見つからない | B の手順 2 の Build Tools が入っていない（または A に切り替える） |
| 日本語が文字化けする | 実行ファイルは起動時に端末を UTF-8 にする。古い「コマンド プロンプト」なら Windows Terminal を使う |

## 実行時エラーの形

ふだんはインタプリタと同じ「借りた → 解放された → 使った」をソースの行つきで出す（ソースを実行ファイルに入れておく）。

```
error: 無効になった参照を使いました (dangling reference)
 --> b.jw:7:11
  |
5 |     let r = ref a;
  |             ----- ここで `a` を借りた
6 |     drop(a);
  |     ------- ここで解放された: `a` が drop で明示的に解放された
7 |     print(*r);
  |           ^^ ここで使ったが、借りた先はもう存在しない
  |
  = note: 参照は、借りた元の所有者より長くは生きられません
```

`--release` では使った位置だけ:

```
error: 無効になった参照を使いました (dangling reference)
 --> b.jw:7:11
  = ここで使ったが、借りた先はもう存在しない
```

- 「解放された」位置は、区画が使い回されたあとも直近 4096 回分は覚えている。それより前だと「解放された」の行は出ない（推測: ふつうのプログラムではまず起きない）。
- 終了コードはエラーなら 1。
