# CLAUDE.md 旧詳細版（2026-10-28保存）

短縮前の全文を以下に保存する。現在の作業指示と安全性の判定基準は、リポジトリ内の `../CLAUDE.md` を優先する。
履歴中のブランチ指定・作業段階・検証状態は、保存当時の記述として扱う。

> **開発段階の優先順位:** 現在は形式証明を実施・監査する**ツール本体の実装**が目的。証明用の例題、ケースマトリクス、追加の負例・回帰テストの量産を作業成果としない。形式証明テストケースの体系的整備はツール完成後に実施する。既存のツール実装とコンパイラ修正を壊す削除はしない。ツールの単体テストが必要な場合も実装に直接必要な最小限に留める。テスト数をツール実装進捗へ算入しない。

> **PR運用の厳守事項:** ドラフトPR（Draft pull request）を作成しない。PRを作る場合は必ず通常のPR（`draft: false`）で作成する。既存のドラフトPRがあれば通常PRへ変更する。CIの自動起動設定は変更しない限り、手動実行のみを維持する。

> **最優先の開発指示（2026-10-27）:** 現在の依頼は「形式証明テストを行うツールの完成」であり「Jabberwock全体の形式証明を成立させること」ではない。ツールの実装完了、ツールのテスト実施状況、検証対象の証明結果は独立して報告する。未証明の項目数・全命令/全IRの未証明数をツールの実装進捗率に用いない。未証明は正しく NOT_PROVEN と返し、証明成功を偽装しない。詳しくは `docs/formal-tool-completion.md`。ツールの進捗報告は `python scripts/tool_implementation_progress.py` を用い、`docs/tool-capabilities.json` の各項目をコード監査で根拠付き更新する。実機テストの未実施は実装未完成と混同しない。

# CLAUDE.md

自作言語 jabberwock の Rust 実装。`own` と世代付き `ref` だけで「C++より安全、Rustより簡単、Goより速い」を狙う。

## 作業ルール（オーナーの指示）

- 変更は **main に直接 push してよい**（PR は不要）。push 前に `cargo test` と `cargo clippy --all-targets` を通すこと。
- オーナーとのやりとりは日本語、平易な言葉で。
- ソースコードについて質問されたら答えられるよう、下の「コードの地図」と不変条件を最新に保つ。
- 外部クレートは使わない。Rust 1.65 以上で動くこと（`is_some_and` など新しい API は避ける）。
- `cargo fmt` はリポジトリ全体ではかけていない（元のコードが rustfmt 形式ではないため）。周りの書き方に合わせる。

## コードの地図

形式化は `formal/`（Lean 4.24.0、抽象的な世代・退役と所有位置間のムーブ）。
`scripts/check_formal.py` で証明と公理依存を検査する。範囲と未証明事項は `docs/formal-semantics.md`。
上限での退役はRust/IR/C/native/jwcに実装。経路・境界試験・未実施範囲は `docs/generation-retirement.md`。
RustはSlot.retiredを空き表へ戻さず、Cは状態bitを除いたcounterの余地を確認して全1markerで退役する。
Cの退役区画はatomicな保持表へ残し、参照検査は世代一致でもmarkerを拒否する。生存中の上限は規定の資源エラーで停止する。

`runtime/jw_rt.c` を変更したら `python3 scripts/sync_runtime.py` で `jwc/runtime.jw` を再生成する。
このコマンドは生成ファイルだけを更新し、ビルドやテストを実行しない。同期の検証は既存の `runtime_is_up_to_date` テストが担当する。
安全性はWindows用12項目・Linux用12項目を独立に扱う。
入口は `scripts/safety_windows.py` / `scripts/safety_linux.py`、OS別一覧は `scripts/safety_platforms.py`。
共通の単一JSON実行器は `scripts/safety_report.py`、旧来の簡易実行器は `scripts/local_safety.py`。
schema 2の `suite` / 項目ID / 出力ファイル名でOSを区別し、Linux（WSL2を含む）の結果をWindowsの合格に流用しない。
資源枯渇8と巨大配列9は別プローブ。Windowsの未対応検出器はNOT_RUNとして項目を残す。
native専用の退役検査はclang必須・`JW_REQUIRE_RETIREMENT_TESTS=1`とし、スキップを成功扱いしない。

| ファイル | 役割 |
|---|---|
| `src/lexer.rs` | 字句解析。改行はただの空白（文の区切りは `;`）。`"""..."""` だけ途中で改行できる（トークンの位置は開きの行） |
| `src/parser.rs` | 構文解析。`if`/`while` の条件では構造体リテラルを禁止（`{` があいまいになるため）。`x += e` は `x = x + e`、`x++` は `x = x + 1` に書き換える（左辺は複製） |
| `src/ast.rs` | 構文木。`Deref` と `Field` は静的解析の結論 `Cell<Check>` を持つ。整数の型 `IntTy`（範囲・一周・自動変換の規則）もここ |
| `src/types.rs` | 型検査。実行前に型と `mut` / `ref mut` の違反を見つける（`lib.rs` で解析・実行の前に呼ぶ） |
| `src/analyze.rs` | 静的解析。証明できた `*p` / `p.f` の ref 世代チェックを消す |
| `src/interp.rs` | 木をたどるインタプリタ。ヒープはスロット＋世代、変数は Live / Moved |
| `src/sort.rs` | 並べ替えの手順（interp と IR の exec が使う。jw_rt.c と同じ順に比べる） |
| `src/diag.rs` | 「借りた → 解放された → 使った」を時系列で出すエラー表示 |
| `src/threads.rs` | スレッド（interp 用）: `World`（出力・待ち合わせ・デッドロック検出・エラー）、`Portable`（ヒープから切り離した値）、`Chan` / `Task` |
| `src/modules.rs` | モジュールの読み込み（`Loader`）と、モジュールの中の名前を `geom.area` に書き換える `qualify` |
| `src/project.rs` | パッケージ（alice.toml の読み取り・依存先とモジュールの場所・`new`）。コマンドは別の実行ファイル `alice`（`src/alice.rs` の `project_*`）。説明は docs/packages.md |
| `src/lib.rs` | `run_source` / `run_source_with`（解析なしで比較できる）/ `run_with_modules`・`render`（複数ファイルのエラー表示）/ `ir_text`（`--ir`）/ `ir_dump`（`--ir-raw`。解析の前の IR、jwc との突き合わせ用）/ `ir_mismatch`（IR との突き合わせ）/ `llvm_build`（`jabberwock build`）/ `native_mismatch`（実行ファイルとの突き合わせ） |
| `src/ir/mod.rs` | IR（中間表現）の形と表示。設計は `docs/ir-design.md` |
| `src/ir/lower.rs` | 構文木 → IR の書き出し。書き出せない書き方は `Err(理由)` |
| `src/ir/exec.rs` | IR を実行する小さなインタプリタ（ヒープ・表示・計算は `Interp` のものを使う） |
| `src/ir/llvm.rs` | IR → LLVM のテキスト（`jabberwock --ll`）。書き出せない書き方は `Err(理由)` |
| `src/native.rs` | clang を呼んで実行ファイルを作る（`jabberwock build`。実行時ライブラリはコンパイラの中に入れてある）。環境変数 `JW_CFLAGS` を clang に足す（`-fsanitize=` の address / thread / memory なら、書き出した関数にも `sanitize_*` を付ける。`instrument`） |
| `runtime/jw_rt.c` | 実行時ライブラリ（区画と世代・文字列・リスト・辞書・出力・エラー） |
| `src/ir/analyze.rs` | IR の上で ref の世代チェックを消す解析（`Proj::Deref` の `Check` を書く） |
| `tests/examples.rs` | 言語の基本動作 |
| `tests/retirement.rs` / `tests/support/retirement.c` | 本当の世代上限と小さい上限、空き表・保持表・送信・リストの退役。生成LLVMのmarker一致の負例は support/retirement.rs、jwc側も同じ負例を実行 |
| `tests/analyze.rs` | 検査を消す解析。消せる例と、消してはいけない例 |
| `tests/structs.rs` | 構造体 |
| `tests/control.rs` | loop / while / for / break / continue / if 式 |
| `tests/match.rs` | match（Java の switch の矢印形式） |
| `tests/types.rs` | 型検査 |
| `tests/enums.rs` | enum と Option・match での取り出し・`match ref` |
| `tests/generics.rs` | ジェネリクス（関数・構造体・enum の型の引数） |
| `tests/results.rs` | Result と `?`（途中で抜けたときの解放も） |
| `tests/methods.rs` | メソッド（`this`・`fn mut`・構造体の値への呼び出し・静的メソッド） |
| `tests/interfaces.rs` | interface（Go 風）と `<T: Shape>` / `<T: ordered>`・interface を型として使う |
| `tests/strings.rs` | 文字列の埋め込み `"x = {x}"` |
| `tests/arrays.rs` | 配列（`i64[3]`・添字・定数の添字の範囲チェック） |
| `tests/lists.rs` | リスト `i64[]`（push / pop / len）と `for x in a` |
| `tests/floats.rs` | 小数 f64 |
| `tests/ints.rs` | 整数型・数の書き方・自動変換・`as` |
| `tests/text.rs` | 文字列のメソッドと入力（`read_line()` / `args()`） |
| `tests/borrow.rs` | own でない値を借りる・`ref` の引数を自動で借りる |
| `tests/typeinfo.rs` | 型検査が式ごとの型・型の引数を記録すること（IR の土台） |
| `tests/ir.rs` | IR の表示と、書き出せる範囲 |
| `tests/native.rs` | clang で作った実行ファイル（clang がなければ何もしない。スレッドも）。区画の確保・使い回し・別スレッドでの解放・`Option<own T>`・文字列の長さを、interp と -O1 / -O2 で突き合わせる（`same_as_interp`） |
| `bench/` | jabberwock・Go・C++・Rust で同じ処理のベンチマーク（`bench/run.sh`、結果は `bench/README.md`）。時間の中央値・最大メモリは `bench/measure.c`、jabberwock は 3 通り（ふつう・`--no-elide`・`--unchecked`）と検査の数（`--stats`・`--count-checks`）、C++ は境界チェックつきも。ビルド時間・大きさ・起動・スレッド数・速くした書き方は `bench/extra.sh`。検査の数の見張りは `bench/checks.sh`（`bench/checks.txt` と比べる。CI の bench-checks） |
| `tests/modules.rs` | モジュール（`use`・`pub`・`geom.area(p)`・エラーの位置） |
| `tests/project.rs` | パッケージ（alice.toml・依存先・`alice new` / `run` / `test` / `build`） |
| `tests/maps.rs` | 辞書 `i64[str]`・`for (k, v) in m`・`for (i, x) in a` |
| `tests/closures.rs` | クロージャ・関数の値・`sort` / `sort_by` / `sort_unstable`（並び方と長さを変えた `sort_patterns.jw` も） |
| `tests/threads.rs` | スレッド（spawn / join / chan・送れる型・デッドロック） |
| `tests/mutability.rs` | `mut` / `ref mut` / `;` / ブロックの値 / 複合代入 / `++` |
| `jwc/` | jabberwock で書いた jabberwock のコンパイラ（設計は docs/self-hosting.md）。`main.jw`（`--tokens` / `--ast` / `--typed` / `--ir-raw` / `--ir`・PRELUDE）・`lexer.jw`・`diag.jw`・`ast.jw`・`parser.jw`・`modules.jw`・`astdump.jw`・`ty.jw`（型）・`typeck.jw`（型検査）・`ir.jw`（IR の形と表示）・`lower.jw`（IR への書き出し。lower.rs と同じ順に同じ IR を書く）・`analyze.jw`（ir/analyze.rs の移植。事実は変数ごとの数 1 つを、分かっている変数だけ番号の順に並べる）・`llvm.jw`（ir/llvm.rs の Module: 型の形・手伝いの関数）・`llvmfn.jw`（FnGen と `emit`。Rust 版と同じ順に同じ番号をつける）。`--ll` も。`jwc build` で実行ファイルまで作れ、jwc 自身もコンパイルできる。`runtime.jw` は runtime/jw_rt.c を文字列にしたもの（手で直さず `python3 scripts/sync_runtime.py` で作り直す） |
| `src/astdump.rs` | 構文木を文字にする（`jabberwock --ast`。jwc/astdump.jw と同じ形）。`--typed`（`dump_typed`）は型検査の結果も足す |
| `tests/jwc.rs` | jwc を実行ファイルにして、Rust 版と出力を突き合わせる（`--tokens` は `lib.rs` の `token_dump`、`--ast` は `ast_dump`、`--typed` は `typed_dump`、`--ir-raw` / `--ir` は `ir_dump`、`--ll` は `llvm_text` と比べる（`same_ll`）。テストのプログラムを集めた `tests/jwc_cases.txt` も使う。clang がなければ何もしない） |
| `tests/fuzz.rs` | 解析あり・なしのAST/IRとネイティブ、jwc生成LLVMをsanitizer付きでも比べる。型の通る攻撃78件と演算の端76件はスキップを認めない。共通入力は tests/support/attacks.rs。差分テスト。種から写し・ジェネリクス・match ref・リストの操作・要素を借りた ref・メソッドの this を組み合わせたプログラムを作り、interp（と IR）と実行ファイルで比べる（`JW_FUZZ=本数`・`JW_FUZZ_SEED`・`JW_FUZZ_JWC=jwc`）。clang がなければ何もしない。**新しい取り出し経路・組み込みの操作を足したら、ここの部品にも足す** |
| `tests/robust.rs` | 堅牢性テスト。コマンド（`--ir`・実行・`build`）を別のプロセスで動かし、極端な入力（深い入れ子・長い鎖・巨大な数など）と、例とテストのプログラムを壊した変異で、panic・スタックあふれ・内部エラー・シグナルで落ちないかを見る（時間切れは可）。メモリ破壊・情報漏洩を狙うプログラム（`ATTACKS`）は interp・解析なし・実行ファイルで必ずエラーで止まり、隠した値を出さないことを確かめる。`JW_ROBUST=本数`・`JW_ROBUST_SEED`・`JW_ROBUST_NATIVE=1`（変異も実行ファイルで）・`JW_ROBUST_ASAN=1`（ASan/UBSan つきで作る）・`JW_ROBUST_DUMP=フォルダ`（落ちた入力を残す）。**落ち方を直したら `hostile` に足す**。`generated_code_guards_undefined_operations` は書き出した割り算・ずらしの守りの形を見る（llvm.rs の守りを変えたら直す）。確かめた範囲と残りは docs/safety-testing.md・docs/safety-validation.md、CI は `.github/workflows/safety.yml`（毎晩は 20 倍） |
| `tests/system.rs` | ファイルの読み書き・`run_command`・`exit`・`eprint`・値なしの値 `()` |

`tests/sanitizers.rs` は生成LLVMのASan/MSan/TSanとCのUBSan/TSanの負例。MSan時はC関数の宣言にnoundefを付け、境界での検査漏れも確かめる。

## 大事な不変条件

- `own` は唯一の所有者。値として読むとムーブ。構造体も値として読むとムーブ（中に own を持ちうるため）。
- 値を捨てるときは `release` で中の own を（構造体のフィールドも含めて）すべて解放する。リークはテストで `live_objects == 0` を確認。
- 変数の値もヒープのスロットに置く（`VarState::Live(slot)`）。ムーブ・スコープ終了で `vacate` して世代を進める（変数を借りた ref が無効になる）。
  `RefVal` は `slot` / `generation` に加えてスロットの中の道筋 `path`（フィールドと添字）を持つ。own の先を指すなら空。
  道筋は辞書の中を通らない（`through_map` / types.rs の `inside_map` / lower の `through_map` で弾く）。enum の中も通らない（`place` が通さない）。
- リストは `Value::List(スロット)`。中身 `Array(_, true)` は own の先と同じく自分のスロットに置く（`new_list`。`Array(_, true)` をじかに値として持たない。
  文字列のメソッドなどヒープなしで作ったものは `adopt` で置き直す）。要素の場所は中身のスロットから数える（`index_place` / exec の `resolve_in` が `list_place` で移る）。
  要素が動く操作（push・pop（中身があるとき）・insert・remove・drain・reverse・sort・sort_by・sort_unstable・sort_unstable_by）は `list_moved` で中身のスロットの世代を進める
  （理由の文は `list_moved_why`。IR の `seq_op`・llvm.rs の `seq_op` も同じ操作で同じ文）。リストの写し（`view_of`）は中身も新しいスロットに写すので、
  写しを捨てるときは `release` する。一時的な値の中から読んだリストは `copy_lists` で写す（一時的な値と一緒に解放されるため）。
  IR の `Operand::Copy` も中のリストを写す（実行ファイルの複製と同じ）。`Show(Copy(場所), false)` は写さずに場所を見る。
  中身のスロットの数も `live_objects` に入る（実行ファイルは数えないが、正常に終われば両方 0）。
  `check_ptr` は (スロット, 道筋) を返し、`at` / `at_mut` で中の値をたどる。
- `ref T` の引数に `ref` なしで場所を渡すと、types.rs の `auto_borrow` が `Expr.borrow` を立てる。interp（`eval` / `eval_borrow`）と
  analyze（`expr`）はそれを `ref e` と同じに扱う（`make_ref` / `ref_of`）。関数とメソッドの引数だけ（enum の中身・構造体のフィールドは対象外）。
  場所でない式（一時的な値）にも立てる。interp は `auto_ref` でスロットに置いて借り、呼び出しを包む `with_lent` が終わったら解放する。
  lower は一時的な変数に置いて `Rvalue::Ref` し、呼び出しのあとで drop（`lent` / `drop_lent`）。analyze は値として評価して事実なし。
- 静的解析の根拠: own 変数 `a` のスロットを解放できるのは `a` という名前を使う操作（再代入・ムーブ・`drop`・スコープ終了）だけ。呼び出された関数は呼び出し元の変数に触れない。
- 解析は安全側。分からないものは `Check::Required`（実行時検査）。デバッグビルドでは消した検査も `debug_assert!` で確かめる。
- 解析とインタプリタの「評価順」と「ムーブする／しない（`eval` と `eval_borrow`）」は必ず一致させる。
- `break` / `continue` で抜けるときは、抜けるスコープの変数の事実を消してから合流させる（`facts_leaving`）。
- `if` / `match` は式（`ExprKind::If` / `ExprKind::Match`）。文として使うときは `Stmt::Expr` に入り、中の return / break を `Flow` で外へ伝える（`eval_flow`）。
  値として使うときの外への break / continue はパーサが禁止している（`value_context`）。return は書ける: interp の `eval` が If / Match で
  `Flow::Return` を受けたら、`unwinding` に返す値を入れ `returning` を立てて `Err` で抜け（`?` と同じ道）、`call_fn` が return にする。
  lower は値の中の深さ `in_value` を数え、そこでの return はまだ使っていない一時的な値を `drop_pending` で捨ててから抜ける。
- 書き換えの可否は `Access`（Write / ImmutVar / ReadOnlyRef）で表す。`place()` が持ち主から順にたどって決め、
  `ref mut` を通ると Write のまま、`ref` を通ると ReadOnlyRef になる。`RefVal.mutable` が「ref mut か」。
  同じ規則を `types.rs` が型から静的に決める（`Access::through` に型を渡す）。実行時の検査は念のため残している。
  二つの規則は必ずそろえる。
- シャドーイングは禁止。types.rs の `declare` が、見えている名前と同じ名前の宣言をエラーにする。
- 型検査は「前から順に決めるだけ」。`ref mut T` は `ref T` として渡せる（`assignable`）。必ず抜けるブロックは `Ty::Never`。
- 戻り値の型がある関数では、本体の最後の `;` のない式が戻り値（`call_fn` が `exec_block_value(body, true)`）。
  戻り値のない関数では最後の式の値は捨てる。
- 整数の値は `Value::Int(i128, IntTy)`。計算は i128 でして、型の幅で一周させる（`IntTy::wrap`）。
- 型検査は整数の式に「この型で使う」という印 `Expr.int` をつける（数の書き方を相手に合わせる・自動の変換）。
  interp は `eval` / `eval_flow` の最後にその型へ変換する（`convert`）。印をつけるのは `types.rs` の `convert` / `set_tree` だけ。
- 小数は `Value::Float(f64)`・`Ty::Float`。数だけの式が f64 の場所にあれば、型検査が `set_float_tree` で `Expr.float` をつけ、
  interp の `convert` が整数を小数にする（中の数すべてにつけるので、計算も小数でする）。
- 整数のあふれは一周する。定数の式（`is_const_tree`）の結果が型に入らないときだけ `types.rs` が警告を出す（そのまま書いた数ならエラー）。
  警告は `Diagnostic::warning` で、実行は止めない（`Outcome.warnings`、CLI は実行前に表示）。
- enum の値は `Value::Enum { variant, fields, copy }`。`copy` は型検査が `ExprKind::Variant` の `Cell<bool>` に書く
  （own や構造体を持ちうる型ならムーブ。`Value::moves`）。型の引数を含んで決められないときは None で、interp が中身を見て決める。
  Option と Result は `lib.rs` の `PRELUDE` を `parse` がプログラムの前に足す。`Some(x)` / `None` / `Ok(x)` / `Err(e)` はパーサが `Option.Some` などにする。
- enum の match は interp の `match_enum`。値の match は中身を受け取る名前へ移し、借りた match（scrut が ref）は
  中身の own を ref にして渡す。全部の種類を書いたかは `types.rs` の `match_enum` が調べる。
  case で受け取る名前は、analyze でも新しい変数として宣言する（外の同名の変数の事実を使わない）。
  形を重ねられる（`Pat::Variant` の 3 つめ。重ねた位置の名前は `_`、全部の名前は `Pat::names`）。interp は `subs_match` で値を動かさずに調べてから
  `bind_fields` で受け取る。types は `variant_pat` / `sub_pat`、全部書いたかは `variant_covered`。lower は case ごとに `test_pat`（外れたら次の case へ）→
  `bind_pat` → 入れ物の drop を **同じブロック** に置く（llvm.rs はそのブロックで take した入れ物の中身を解放しないため）。
- パーサはファイル中の `enum 名前` を先に集め、`名前.種類` を enum の値として読む（`enum_names`）。
- ブロックの値は `Block.tail` が true のときだけ（最後の文が `;` のない式）。`}` で終わる文（`is_block_like`）には `;` 不要。

- ジェネリクス: 型検査だけの話で、interp は型を見ない（値はそのまま動く）。`Ty::Param` は定義の中の `T`、
  `Ty::Struct/Enum(名前, 型の引数)`。使う側では `instantiate` が引数（ふつうの式 → 数の式 → enum の値の順）と
  入れる先の型から `unify` で型の引数を決め、`subst` で置き換える。`T` は copyable ではない（安全側）。

- `e?`（`ExprKind::Try`）で抜けるときは、interp が `unwinding` に返す値を入れて `Err` を返し、`call_fn` が受け止めて return にする。
  途中のスコープは frame に残ったままなので `call_fn` がまとめて解放する。評価途中の値（引数・構造体のフィールド・print の一時値）は
  `eval_all` / `discard` で解放する。新しく「値を集めながら評価する」場所を足すときは、同じように解放すること（リークする）。
  analyze では `?` で抜ける道は関数の中で合流しないので、続く道だけ考えればよい。

- メソッドは、パーサが `構造体名.名前` という普通の関数にする（最初の引数 `this` は `ref [mut] 構造体<T..>`、
  型の引数は構造体のもの＋メソッドのもの）。本体のフィールド名・メソッド名は `bind_this` が `this.名前` に書き換える
  （引数・ローカル変数にフィールドと同じ名前はつけられない。types.rs の `not_a_field`）。呼び出しは `ExprKind::MethodCall`。
  interp の `eval_method`: 一時的な receiver は先に評価、場所の receiver は引数のあとで借りる（analyze も同じ順）。
  場所にある構造体の値の receiver はその場所を借りて this にする（リストの要素も）。一時的な値と辞書の中の receiver だけヒープへ貸し（`Lent`）、呼び出し後に解放して元の場所へ戻す。
  analyze は receiver の型を知らないので、同じ名前のメソッドすべての呼び出しとして記録する（安全側）。
  静的メソッド（`static fn`）は this のない `構造体名.名前` の関数。パーサが `struct 名前` を先に集め（`struct_names`）、
  `Point.at(...)` を `ExprKind::Call("Point.at")` にする。型検査の `FnSig.method` で呼び方の取り違えを弾く。
  enum のメソッドも同じ（`parse_enum` が種類のあとの `fn` を読み、`parse_method` / `finish_methods` を構造体と共有）。
  enum の値は型の名前を持たないので、型検査が `MethodCall` の 4 つめ（`RefCell<Option<String>>`）に enum の名前を書き、
  interp の `eval_method` はそれで `Enum.名前` を探す。`Shape.x(...)` は x が種類でなければ静的メソッド（パーサの `enum_variants`。
  モジュールの enum は `ModuleInfo.variants`）。enum は interface に当てはめられない（`satisfies`）。
  Option / Result の `is_some` などは `lib.rs` の `PRELUDE` に jabberwock で書いたメソッド（prelude の変数名は `_v` など、利用者の関数名とぶつからない名前にする）。
  `fn take` のメソッドは this が値（`ref` でない型）。型検査が `MethodTarget.take` を立て（receiver は own / ref でない値だけ）、
  interp の `eval_take_method`・analyze・lower は receiver を **引数より先に** 値として読む（ムーブ）。interface には当てはまらない。

- interface と条件: 型検査だけの話（interp は値の構造体名でメソッドを探すので何もしない）。`FnDef.bounds` が型の引数ごとの条件。
  `types.rs` の `satisfies` が「構造体のメソッドが interface の形とそろうか」を調べ、`check_bounds` で呼び出しの場所で確かめる。
  関数の中では `Checker.bounds` を見て、`T` のメソッド呼び出しと比較（`is_ordered_param`）を許す。
  interface を型として使うときは `Ty::Iface`（own / ref の先にだけ書ける。`resolve_pointee`）。`own S` → `own I`、
  `ref S` → `ref I` は `Checker::fits`（`assignable` に interface を足したもの）で許す。interp は何もしない（値の構造体名でメソッドを探す）。
  メソッド呼び出しは `T` も interface も `iface_method_call`。if / match の枝は、入れる先が interface なら `join` がそれにまとめる。

- 文字列の埋め込み: lexer が `{...}` の中を別に字句解析して `Tok::Interp`（位置は元のファイルに直す）、
  パーサが同じ enum / 構造体の名前で式に読む（`ExprKind::Interp`）。値は print と同じく `eval_borrow` で借りて読む（analyze も borrow）。

- 文字列のメソッドは types.rs の `str_method`（型）と interp の `str_method`（動き）。長さと位置は文字単位。
  print と埋め込みの表示は `Interp::show`（own / ref は中身を出す。ref は世代チェックする）。
  入力は `Interp::set_io` で渡す（CLI は標準入力、テストは `run_source_with_input`）。`read_line` / `args` は組み込み関数（`BUILTINS`）。

- 配列: `Value::Array`（要素にムーブする値があればムーブ）。`Place.path` は構造体のフィールドと配列の添字の両方をたどる。
  `a[i]`（`ExprKind::Index`）は **添字を先に評価してから** 配列の場所を求める（interp の `place` / `eval_borrow`、analyze も同じ順）。
  定数の添字の範囲チェックは `types.rs`、実行時は `bounds`。`len()` は `eval_method` / `method_call` の特別扱い。
  リストは `Value::Array(_, true)`（いつもムーブ）・`Ty::List`。`[...]` がリストかは型検査が `ArrayLit` の `Cell<bool>` に書く。
  `push` / `pop` は `eval_method` が場所の中のリストをじかに書き換える（丸ごと複製しない）。
  `for x in a`（`Stmt::ForEach`）は `eval_borrow` で借り、始めた時点の要素を回す。own の要素は `RefVal` にして渡す。
  ただし、その場で作ったリスト（`a.drain()` など。型検査が `ForEach.consume` を立てる）は要素を値で受け取る（interp の `exec_for_take`・lower の `for_take`。
  逆順にして後ろから pop する。analyze はその iter を eval で読む）。
  構造体などムーブする要素は、写し（`view_of` で中の own を読むだけの ref にしたもの）を一時的なスロットに置いて ref で渡し、周回ごとに `vacate` する。
  写しの中の own（own の配列）を値として取り出すのはエラー（型は own なのに中身が写しになるため）。型検査が `p.f` / `a[i]` / `*p` の型を見て `Expr.takes_own` を立て、
  interp はそれを値として読むとエラー（型の引数 `T` の場所は型で分からないので、`view_of` が写しの ref に付ける印 `RefVal.view` を `is_view` で見る。
  借り直した ref（`make_ref`）と、ref の型で受け取る match ref・for・map・sort_by の要素は印を消す（`unview`）。IR の exec は copy_out の行き先の型（`holds_own`）、llvm.rs / jwc の Hazard は own なら必ず真。

- モジュール: `modules::Loader` が `use` をたどってファイルを読み（使われる側が先。輪はエラー）、各ファイルを
  `Parser::with_modules` で解析する。パーサは `geom . 名前` の 3 トークンを 1 つの名前 `geom.名前` にまとめ（`merge_paths`。pub でなければエラー。`.` のすぐあとの名前はフィールドなのでまとめない）、
  モジュールの中の定義と参照は `qualify` が `geom.名前` に書き換える。全部を 1 つの `Program` にまとめるので、types / analyze / interp は
  モジュールを知らない（types だけ、モジュールの中で関数と同じ名前の変数を弾くために `Checker.module` を見る。メソッド名は `rsplit_once('.')` で分ける）。
  `Span.file` がファイルの番号（`Outcome.files` の位置）。表示は `jabberwock::render`。
  パッケージの名前の空間: 読む関数への問い合わせは `名前`（自分のパッケージ）/ `空間*名前`（依存先の lib.jw。最初のパッケージからは `*名前`。
  キーは `名前` で `Loader.libs` に入れる）/ `P/名前`（依存先 P の中のモジュール。キーも `P/名前`）。依存先 P の中からは `P/u` → `P*u`、それ以外からは `u` → `*u` の順に試す。
  `空間*名前` は読んだことがあっても毎回問い合わせる（project.rs の `reader` が `direct` で、直接の依存先かを確かめる）。
  書いた名前とキーが違えば `Qualify.alias` が `u.f` を `P/u.f` に書き換える。`Outcome.files` の名前は `名前` / `*名前` / `P/名前`（スレッドの `reloader` は
  `空間*名前` を `*名前` で探す）。表示するときは `*` を外す。project.rs の `modules` のキーも同じ形。
- IR への準備（設計は `docs/ir-design.md`）: `types::check_with_info` が `TypeInfo` を返す。式の型（`expr_ty`）と、
  呼び出し・構造体 / enum の値の式で決めた型の引数（`targs`。`instantiate` が `pending_targs` に置き、`expr_ctx` が受け取る）。
  キーは構文木の中の式のアドレスなので、`Program` を書き換えないこと。数だけの式（`is_const_tree`）は根だけ記録する。
  式をたどるのは `ast::walk_block` / `walk_expr`。
- 入れ子の深さ: パーサの `nested`（中で `deeper`）が `parse_block` / `parse_pat` / `parse_type` / `parse_unary` に入るたびに数え、`MAX_NEST`（256）を超えたらエラー。
  演算子・`as`・後ろにつく `.` / `[` / `?` の鎖も 1 つごとに `deeper` で数える（後ろの段は鎖を左に深い木として再帰でたどるため。抜けるときに元の深さへ戻す）
  （本体は `*_in`。埋め込みの中の式を読む sub の Parser も深さを引き継ぐ。jwc/parser.jw も同じ数え方で同じエラー）。後ろの段の再帰はこれで抑える。
  コマンドの `main` は `main_in_big_stack` で全体を 1 GiB のスタックで動かす。文字列の長さの上限は `STR_MAX`（2^62。interp と jw_rt.c）。
  固定長の配列は 1 MiB まで（types の `check_size`。大きさは `cells` が 8 バイト単位で数え、`CELLS_CAP` で頭打ちにする。jwc/typeck.jw も同じ数え方。
  Rust 版は読み終わっていない enum を種類なしに見るので、jwc は `EInfo.ready` でそろえる）。
- 深い再帰（呼び出しの深さ 5000）で Rust のスタックがあふれないよう、`exec_stmt` / `eval_kind` の枠を大きくしない（大きな処理は `#[inline(never)]` の別関数へ。`exec_for_each` など）。
  テストは `cargo test --no-fail-fast` で全部のファイルを走らせて確かめる（スタックあふれは途中で止まり、後ろのテストが走らない）。
- 辞書: `Value::Map(Box<MapVal>)`（入れた順の `entries` と、キーから位置を引く `index`）・`Ty::Map(キー, 値)`・`TypeKind::Map(値, キー)`。
  `["a": 1]` は `ExprKind::MapLit`、空の `[]` は型検査が `ArrayLit` の `Cell<Seq>` を `Seq::Map` にする。
  `Place.path` の要素は辞書なら `entries` の位置（`at` / `at_mut`）。辞書の中は `through_map` / `inside_map` で借りるのを弾く。
  `a[k]` は interp の `index_place`（`locate` で位置を求める）。代入 `m[k] = v` だけ `create` でないキーを入れる（仮の Unit を置いてから置き換える）。
  型検査は `a[k]` の base を先に調べる（キーの型を入れる先として使う）。interp / analyze は添字が先のまま。
  `for (k, v) in m` / `for (i, x) in a` は `Stmt::ForEach.key`。
- クロージャ: `ExprKind::Lambda(Rc<Lambda>)`、型は `Ty::Fn(引数, 戻り値)`。types の `lambda` が引数の型を書いた型か入れる先の型（hint）から決め、
  本体で使った外の変数を `use_var` で集めて `Lambda.captures` に書く（`lambdas` に (外のスコープの数, 名前) を積む）。取り込んだ変数への代入は `access` で弾く。
  interp の値は `Value::Func(Callee, 取り込んだ値)`。作るときに captures の順で `read_var`（own はムーブ）。
  呼ぶとき（`call_func`）は取り込んだ値を `view_of` の写し（own → 読むだけの ref）で新しいフレームに置き、ムーブする値だった変数は `Var.view` で取り出しを禁止する。
  `f(x)` は、`f` という変数があれば関数の値の呼び出し（`call_var`。引数が先、変数はあとで借りる）。変数に関数と同じ名前は禁止（`not_a_fn_name`）。
  関数・メソッドの引数にその場で書いたクロージャはパーサが `Lambda.borrows` を立て（spawn は除く）、types の `use_var` がムーブする値
  （`borrowable`）の取り込みを `Lambda.borrowed` に記録して、中では `ref T` に見せる。interp（`make_ref`）・analyze（事実を消さない）・lower（`Rvalue::Ref`）は
  その取り込みを `ref 名前` として作る。
  名前のある関数を値にすると `Callee::Named`（analyze は全引数を Pinned でないと記録）。analyze はクロージャの本体を、何も分からない別の文脈で調べる。
  `map` / `filter` は interp の `map_list`、lower は for と同じ `each_in`（`Each::Map`）で書き出す（exec / llvm は何もしない）。
  `ok_or` は `lib.rs` の `PRELUDE` に jabberwock で書いた関数。
  `str[]` の `join(sep)` は interp の `join_strs`・llvm の `jw_list_join`（元のリストは変えないので mut でなくてよい）。
  `sort` / `sort_by` / `sort_unstable` / `sort_unstable_by` は interp の `sort_list`（要素の写しで比べて順番を決め、最後に元の要素を並べ替える）。
  順番は `src/sort.rs`（安定: run ＋二分挿入ソート＋powersort、安定でない: pdqsort）。jw_rt.c の `jw_sort` / `jw_sort_unstable` と
  **同じ手順・同じ順に比べる**（比べる関数の呼ばれ方が見えるため）。片方を変えたらもう片方も同じに直す。
  例外: 実行ファイルの整数の `sort()` / `sort_unstable()` は比べない基数ソート（`jw_sort_int`。llvm.rs / llvmfn.jw の `sort_int`。呼ばれ方が見えず、結果も同じ）。

- IR: `ir::lower::lower(prog, info)` が main から呼ばれる関数だけを書き出す（`Proj::Deref` はいつも `Check::Required` で書き出す）。
  **デバッグビルドでは `run_inner` が、書き出せたプログラムを IR でも実行し、出力・エラーの題と位置・残ったヒープの数を interp と突き合わせる**
  （違えば panic。全テストが自動で突き合わせになる）。interp の動きを変えたら lower / exec も同じに直すこと。
  書き出しの決まり: 式の値はいつも一時的な変数に入れ、一時的な変数はちょうど 1 回 `move` で読む。名前のある変数は、ムーブする型
  （own・構造体・リスト・辞書・コピーできない enum）なら `move`、ほかは `copy`。関数の値とクロージャが取り込んだ変数だけは interp と同じく値で決める
  （`Operand::Read`）。ポインタの先・フィールド・要素を値として読むときも値で決める（`CopyOut`。ムーブする値ならエラー）。
  print と埋め込みは `Show`（場所なら借りて読む、一時的な値なら見たあと捨てる）。スコープを抜ける道ごとに内側から `drop`
  （ムーブ済みなら何もしない）。`for range` / `for x in a` は隠れた変数（名前が空）をスコープに置く。`?` で抜ける道は、まだ使っていない一時的な値も drop する。
  ジェネリックな関数は型の引数ごとに作り分ける（`Calls::index`。名前は `pick<i64>`）。クロージャの本体は別の関数（`main::λ行:列`）で、
  取り込んだ値は呼ぶときに `view_of` の写しで最後の引数として渡す（`Callee::Ir`）。interface を型として使ったメソッド呼び出しは、
  その名前のメソッドを持つ構造体の表（`CallDyn`）。match の中身は `take`（`(x as Some).0`）で受け取る。添字は `Proj::Index`（先に計算した添字を持つ）。
  入力（`read_line` / `args`）は組み込み。入力を読むプログラムは、テストが入力を渡したときだけ突き合わせる。
- LLVM（`ir::llvm::emit`）: 値の形は docs/ir-design.md の「7 で決めたこと」。**`JW_NATIVE=1 cargo test` で全テストを実行ファイルでも突き合わせる**
  （`lib.rs` の `native_mismatch`。clang が要る）。IR の動きを変えたら llvm.rs も同じに直し、これを走らせること。
  own の写し（下の 1 ビット）・interface の構造体の番号・変数の状態（null / `@jw.unset` / ムーブした位置）が要。
  実行時ライブラリの関数を足したら、llvm.rs の `PRELUDE` の `declare` も足す（jwc/llvm.jw の PRELUDE も）。jw_rt.c を直したら jwc/runtime.jw を作り直す（`python3 scripts/sync_runtime.py`）。
  実行時エラーは interp の `render_in` と一字一句同じに出す（突き合わせは全文を比べる）。interp のエラーの文を変えたら llvm.rs / jw_rt.c も直す。
  測定用の切り替えは `llvm::Opts`（`emit_with`・lib.rs の `llvm_build_with`。世代の検査・添字の検査を消す、通った回数を `@jw.count.*` に数えて `jw_main` の正常終了時に出す）。ふだんは `Opts::default()` で、jwc は持たない。
  `begin_error` → `put_site`（副の位置）→ `finish_error`（主の位置と note）。ソースは `emit` の `sources`（空なら `--release` の短い形）。Rust 1.65 で `div_ceil` は使えない（`ceil_div`）。
- IR の解析（`ir::analyze::analyze`）: 書き出しのあとに呼び、`Proj::Deref` の `Check` を書く（場所の最初の Deref だけ消せる）。
  事実は局所変数ごとに `Own`（own の持ち主）/ `RefTo(x)`（own 変数 x の先を指す）/ `RefSlot(x)`（変数 x のスロットを指す。再代入では消えない）/ `RefElem(x)`（x から届くリストの要素を指す。
  要素が動きうる操作（リストの操作・呼び出し・組み込み・リストを持ちうる値の代入や drop）で `kill_elems` がまとめて消す。呼び出しの引数の根拠にはしない。
  例外: 取り込みがなく、引数と戻り値がすべて数・小数・真偽・文字列・値なしの直接呼び出し（`scalar_call`。jwc/analyze.jw も同じ）は消さない。
  根拠は「可変のグローバル変数がない」こと。グローバル・FFI・ポインタを作るキャストを足したら見直す。ref は may_hold_list が false でも対象外。docs/scalar-call-handoff.md）/ `Pinned`。
  前向きのデータフローで、合流は共通部分。引数が Pinned かは関数をまたいで固定点まで回す（呼ばれない関数・関数の値の引数は何も仮定しない）。
  **根拠: own 型の局所変数にはいつも own が入っている**（自動で借りた一時的な値・`for` の要素の写しは `ref` 型にする。exec が debug_assert で確かめる）。
  念のため copy_out の結果には Own を付けない（写しの own は取り出しでエラーになる）。
  消せた数は AST の解析以上（テストの 348 本すべてで、AST 110 箇所 → IR 451 箇所。数える場所の数は IR のほうが多い）。
- スレッド（interp）: 型は `Ty::Task(R)` / `Ty::Chan(T)`。送れる型かは types.rs の `sendable`（spawn の取り込み・戻り値・chan の T）。
  interp はスレッドごとに別の `Interp` を本物のスレッドで動かす。構文木は Rc を含むので、新しいスレッドは `World.load`（lib.rs の `reloader`）で
  ソースから読み直し、クロージャは式の位置（`Entry::Lambda(span)`）で探す。値は `export`（own のスロットは「ほかのスレッドへ送られた」で解放）→
  `Portable` → `import` で運ぶ。spawn の関数は 1 回だけ呼ぶので、取り込んだ値は写しにせずそのまま置く（`run_entry`）。
  待つ操作はすべて `World` の 1 つの錠と条件変数（`wait`）。状態が変わるたびに `epoch` を進めて `blocked` を 0 にするので、
  `blocked >= alive` がデッドロック。エラーは `World.fail` → `aborted` を見て、ほかのスレッドは関数呼び出し・ループ・待ちで抜ける（`aborted_marker`）。
  print の出力は `World` に行ごとに足す（`Interp::output()`）。捨てる task は `settle_tasks` で終わりを待つ。
  IR では `spawn`・`chan`・`task.join`・`chan.send` などは `Rvalue::Builtin`。exec は `start_thread` を共有し、新しいスレッドは `World.load_ir`（lib.rs の `ir_reloader`）で IR を作り直して、
  関数の番号で 1 回だけ呼ぶ（取り込んだ値は写しにしない）。lower は spawn に渡すクロージャを `once` で書き出し（取り込んだ値はそのままの型の変数）、
  関数の値は作らずに `spawn!(関数の番号, 取り込む値...)` にする。
  デッドロックは、どのスレッドが見つけても main が自分の待っている位置で知らせる（`IS_WORKER`。表示を毎回同じにするため）。
  スレッドを作れなければ spawn の位置でエラー（`start_thread` / jw_rt.c の `jw_spawn`。錠を持ったまま作り、作れてから `alive` を足す）。
  環境変数 `JW_MAX_THREADS` で上限を下げて起こせる（`threads::max_threads` / jw_rt.c の `max_threads`。`--release` の実行ファイルは見ない）。
- ファイル・コマンド・終了（`exit` / `read_file` / `write_file` / `run_command`）: 型は types.rs の `call`、動きは interp の `system`（IR の exec も同じものを呼ぶ）、
  LLVM は jw_rt.c の `jw_read_file` などと `result_of`（`Result<T, str>` を作る）。エラーの文は OS の文を使わず、interp と jw_rt.c で一字一句そろえる。
  `exit` は `World.exit`（終了コードを覚えて `exit_marker` で全スレッドを止める）。`run_inner` / `ir_mismatch` がそれをエラーでなく終了として扱う。
  exit のあとは片づけないので、残ったヒープの数は比べない。lower は exit の呼び出しのあとを `Term::Unreachable` にする。
  `eprint` は print と同じ形で `World.errput`（`Outcome.errput`）へ。突き合わせは標準エラーの先頭が errput と同じかを見る。
  デバッグビルドは IR でもう一度動かすので、ファイルやコマンドの副作用も 2 回起きる（テストは何度起きても同じになるように書く）。
  `()`（`ExprKind::Unit`）は `Ok(())` のための値。types の `value` と引数の検査は `()` と書いた式だけ値なしを許す。
- 文字列の値は `Value::Str(Rc<String>)`（作るのは `text_rc`）。`s = s + e`（`s += e`）で s が文字列の変数なら、interp の `assign` は
  e を先に計算して `Rc::make_mut` でその場で足す。lower も同じ形を `append!(s の場所, e)` にし、llvm は変数の場所を `jw_str_append` に渡す
  （数が 1 ならその場で足し、数を 2 にして返す。古い値の drop で 1 に戻る）。そのため jw_rt.c の文字列の区画の大きさは長さだけで決まる
  （`str_bytes`。大きいものは 2 のべき乗）。e は s を書き換えられない（呼ばれた関数は呼び出し元の変数に触れない）ので、順番を変えても同じ。
- `const`: パーサの `resolve_consts` が、関数の中で定数の名前を使っている場所を値の式に置き換える（`ast::walk_block_mut`）。
  なので types より後ろは定数を知らない。`Program.consts`（モジュールの中は `geom.N`）は、変数に同じ名前をつけるのを types の `not_a_fn_name` が弾くためだけ。
- ビット演算: `>>` は字句解析では `>` 2 つ（型の `>>` と区別）。パーサの `parse_binary` が、すき間なく並んだ `>` 2 つを `BinOp::Shr` にする。
  ずらす数は左と別の型でよい（types の `binary` が先に扱う。llvm.rs の `shift`）。
- 数のメソッド: f64 は `to_bits` / `sqrt` / `abs` / `max` / `min`、整数は `abs` / `max` / `min`（types の `num_method`・interp の `num_method`（exec も使う）・
  lower の `f64.名前` / `int.名前` の Builtin・llvm.rs は小数が intrinsic、整数が icmp + select。jwc の typeck / lower / llvmfn も同じ）。
- 自己ホストに向けて足りないものの一覧は `docs/self-hosting.md`。
- jwc（`jwc/`）: Rust 版の段ごとに、同じ出力を出す jabberwock のプログラムとして書く。突き合わせ用の出力を Rust 版にも足す（字句は `--tokens` / `token_dump`）。
  トークンは `Token { kind, text, num, span }`（kind は記号・予約語ならその字）。埋め込みのある文字列は平らに並べる（interp → lit / code と中のトークン → end）。
  違い: jwc は ASCII でない文字をすべて名前に使える文字とする（Rust 版は `is_alphabetic`）。エラーの文は Rust 版と一字一句同じにする。
  位置は 1 つの i64（`diag.span_in`。ファイル 9 ビット・行 20・列 16・長さ 18）。構文木（ast.jw）の enum の中身は、借りた match で受け取れるよう
  コピーできない値をすべて `own` で持ち、「あるかないか」は bool と空の値の組。jwc の構文解析は構文木を書き換えない:
  メソッドの本体は構造体・enum の中身を 2 回読んで（1 回目でメソッドの名前を集める）読みながら `this.名前` にし（`locals` / `marks` で見えている名前を数える）、
  定数はプログラムを 2 回読んで（1 回目で値を決める）使う場所を値にする。引数にその場で書いたクロージャは `Program.borrow_lambdas`（位置）に記録する。
  モジュールは modules.jw（Rust 版と同じ順に読み、`Qualify` で名前を書き換えた写しを作る）。`jabberwock --ast` と一字一句同じ出力にする。
  型検査（typeck.jw）は types.rs と同じ順に調べて同じエラーを出す。構文木に書けないので、結論は式の番号（`ast.number` が前から順に振る `Expr.id`）で引く
  表 `typeck.Info` に書く（Rust 版の `Cell` と `TypeInfo`）。入れる先の型（Rust 版の `Option<&Ty>`）は `Hint { some, t }`。
  モジュールの名前と同じ名前の変数は作れないので、型は `ty.jw`、型検査は `typeck.jw`（`types` は変数名に使っている）。
- スレッド（LLVM）: 区画の空きリスト・`jw_live`・`jw_depth`・解放の輪の表はスレッドごと（`_Thread_local`。LLVM 側は `thread_local(initialexec)`）。
  生きている区画の数は全スレッドの合計で見る（終わったスレッドの分は `g_live_done`）。待ちは `g_lock` / `g_cv` 1 組（interp と同じ数え方）。
  送る値は `H::Send` で準備する（own の区画は世代を進めて「ほかのスレッドへ送られた」、文字列は複製）。task の写しは下の 1 ビットを立てたポインタ（解放しない）。
  チャンネルのメソッドは場所のチャンネルを複製せずに読む（数を取り合わない）。出力は print 1 回のあいだ錠を持つ（`jw_print_begin`）。
  錠と条件変数は Linux は pthread、Windows は SRWLOCK（windows.h なしで宣言）。
- 区画（LLVM）: 世代はCでも生成LLVMでもatomicのrelaxed load/storeで読む・書く（送信元の無効なrefが別スレッドで読むことがある。書き手は唯一の所有者だけ）。jwc/llvm.jw・llvmfn.jwも同じ。頭は世代だけの 8 バイト（llvm.rs の `-8`）。世代が奇数なら解放済みで、中身の 2 語目に解放した位置（`FREED_SITE`）。
  小さい区画は塊から切り出し、8 バイト刻みの空き区画の鎖で使い回す。送った区画は世代を 2 進め、位置は `sent` の輪の表。
  「中身なし」と「own 1 つ」の 2 種類の enum はポインタ 1 つ（`Module::niche`。種類の番号と中身の場所は `enum_tag` / `enum_payload` を通す）。
  リストの中身も区画の頭（世代）を持つ。要素を借りると頭の世代に `2 | LIST_SHARED` の印を付け（llvm.rs の `borrow_gen`）、
  `jw_list_moved` は印があるときだけ世代を進めて理由を記録する。一度も借りていないリストは malloc / realloc / free で扱い（前に 16 バイト、`LIST_PAD`）、
  借りたことのあるリストは伸ばすとき新しい区画へ写して古い区画を `block_free` する（古い ref が頭を読めるように）。大きい区画は 64 バイト境界にそろえる。
  呼び出しの深さは関数の最初の引数（`FnGen.depth`）。リストの頭と要素の中の数には TBAA の印（`TBAA_HEADER` / `TBAA_ELEMENT`）。
  最適化に頼る書き出しを変えたら `JW_NATIVE=o2 cargo test`（-O2 と LTO）も走らせる。

## 設計の決めごと

文法や仕様で決めたことは `docs/design-decisions.md` に記録する。
今の言語の決まりは `docs/spec.md`（言語仕様）にまとめる。仕様を変えたら同じコミットで spec.md も直す。

## 安全性検査の作業メモ

jwcを最優先に12項目×コンパイラ＋5目標の実用検査を分離。形式証明は `scripts/formal_report.py`。仕様と未検証範囲は `docs/compiler-safety-matrix.md`。テスト実行は利用者が担当し、結果から修正・PRを行う。今回の改修は未実行。

### 項目3・4・6の検出器（暫定）

既存ツールによる対応は仮。後でJabberwock専用の競合・未定義動作・未初期化検出器を実装する。
WindowsはInspector/UBSan+LLVM verifier/Dr. Memoryの接続口、LinuxはTSan/UBSan+LLVM verifier/MSan。
未導入・非対応・負例検出失敗を合格にしない。詳細と実行コマンドは `docs/provisional-detectors.md`。
専用版は暫定版との検出能力比較後に置き換え、コンパイラ別12項目と5目標の区別を保つ。

## 仕様に基づく安全性判定（2026-10-28）

① `scripts/safety_scope.py`、② `scripts/safety_correspondence.py`、③ `scripts/safety_acceptance.py` を追加。
`python scripts/formal_tool.py` で独立再検証とjwc/Rust別5目標の受入判定を実行する。
仕様上の非違反と、実装で必要な保護条件を区別する。詳細・コマンドは `docs/safety-judgement-mechanisms.md`。
利用者の指示により公開Actionsで既存テストと判定機構を検査する。
工程の正常終了と実装の証明成立は別。未解決の条件・合成証明を合格扱いしない。
