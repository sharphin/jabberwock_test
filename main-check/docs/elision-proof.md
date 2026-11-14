# 検査を消す判断の証明（`formal/JabberwockFormal/ElisionSoundness.lean`）

`src/ir/analyze.rs`（と同じ規則の `jwc/analyze.jw`）が「この `*r` は世代の検査を消してよい」と判断したら、
その時点で `r` は本当に生きている（借りたときの世代と、区画の今の世代が等しい）ことを Lean で証明した。
`sorry` も追加の公理も使っていない（`lake build` が `warningAsError` で通る）。

## 証明したこと

| 定理 | 内容 |
|---|---|
| `elided_access_is_valid` | 事実が正しければ、`safeDeref`（`place` の判断）が真の `*r` は必ず通る |
| `step_sound` | 命令の前に事実が正しければ、解析の規則（`transfer`）を通したあとの事実も、命令のあとで正しい（14 種の命令すべて） |
| `sound_of_sub` / `meet_sub_left` / `meet_sub_right` | 合流（`meet`）で事実を減らしても正しいまま |
| `stable_of_meet` | 繰り返しが止まった（どの辺でも `meet(entry, out) == entry`）なら、各ブロックの入口の事実は前のブロックの出口に含まれる |
| `path_sound` / `elided_access_in_function` | 関数の入口で事実が正しく、解析が止まっていれば、どのブロックのどこで消した `*r` も必ず通る |
| `live_arg_is_valid` | 呼び出しの直前に `is_live` が真の引数は生きた ref で、リストの要素でもない（呼ばれた関数が Pinned と見なしてよい） |
| `old_argument_check_is_unsound` | 以前の引数の判定（読む前の事実を取っておく）の反例 |
| `verify_elided` / `verify_live` / `verifyFast_sound`（ElisionCheck・ElisionCheckFast） | 検査器が通した関数では、消した検査は必ず通り、Pinned の引数には生きた ref が渡る |
| `callee_preserves_caller`（OwnershipFrame） | 呼ばれた関数は、呼び出し元の変数の場所と own の変数の先の世代を変えない（前提 1・2） |

## 模型の考え方

区画に「種類」を付ける。種類は、その区画を解放・使い回しできるのが誰かを表す。

| 種類 | 区画 | 変えられる命令 |
|---|---|---|
| `root x` | own の変数 `x` が持つ先 | `x` の drop・ムーブ・入れ直し |
| `slot x` | 変数 `x` そのものの場所 | `x` の drop・ムーブ（入れ直しでは変わらない） |
| `list` | リストの中身 | 要素を動かしうる操作・リストを持ちうる値の上書きや解放 |
| `nested` | 値の中の own の先・受け渡し中の値 | 何でも（事実はここを指さない） |
| `caller` | 引数の ref の先（呼び出し元のもの） | 関数の中では誰も変えない |
| `free` | 空き・呼ばれた関数の中のもの | 何でも |

命令ごとに「どの種類の区画の世代と種類を変えてよいか」を決める（`Exec`）。解析の規則（`transfer`）は analyze.rs の関数を一つずつ写した。

| Lean | analyze.rs |
|---|---|
| `killAll` / `killValue` / `killElems` / `isLive` / `meet` | 同名の関数 |
| `safeDeref` | `place` の `safe` |
| `setFact` | `Inst::Assign` の最後（自分を借りた事実は入れない） |
| `drop x holds` | `Inst::Drop`（`holds` は `may_hold_list`）。`Inst::Discard` は値の読み（`moveOut` など）のあとに、`holds` なら `elems`、そうでなければ `pure` |
| `moveOut x` | `Operand::Move` / `Operand::Read` |
| `assignOwn x holds` | own の型の変数への代入（`copy_out` 以外）。値は先の命令（`moveOut` など）で読み終わっている |
| `assignNone x holds` | 事実の付かない代入（`holds` は射影なしなら `may_hold_list`） |
| `copyRef` / `moveRef` | ref の変数の `copy` / `move`、`ref_fact` の「`follows` で射影なし・own でない」場合 |
| `borrowOwn r y` | `ref_fact` で根が Own の場合（`RefTo`） |
| `reborrow r s live` | `ref_fact` で根が ref の場合（`live` は `*s` を通る形で `is_live` を求める） |
| `borrowSlot` / `borrowElem` | `ref_fact` の `RefSlot` / `RefElem` |
| `writeInside holds` | 射影のある場所への代入（`holds` は `value_may_hold`） |
| `elems` | `kill_elems` を呼ぶ操作（ふつうの呼び出し・関数の値・`CallDyn`・リストの操作・入出力など） |
| `pure` | `kill_elems` を呼ばない操作（数の計算・`scalar_call`・純粋な組み込み） |
| `access r` | `place` で先頭の `*` を判断する所 |

## 前提

前提 1・2 は `formal/JabberwockFormal/OwnershipFrame.lean` で、持ち主の木の模型と下の「型の決まり」から証明した。3 は一覧を安全側の形にした。4・5 は検査器が 1 本ずつ確かめる。6 は範囲外。

1. **唯一の持ち主**（証明済み・`root_freed_only_by_its_slot`）: own の先を解放・使い回しできるのは、その own を持つ変数の場所そのもの
   （`(slot x, [])`）への操作だけ。ムーブした own の先は受け渡し中（`nested`）になる。
2. **呼び出しの枠**（証明済み・`callee_preserves_caller`）: 呼ばれた関数がどう動いても、呼び出し元の変数の場所（`slot`）と
   own の変数の先（`root`）の世代は変わらない。引数の先（`caller`）は、さらに上の呼び出し元の場所か先なので、
   `slots` に呼び出しの列のすべての変数の場所を入れれば同じ定理で言える。呼ばれた関数の中の区画は、呼び出し元からは `free` と見る。
   - 型の決まり（証明の外。型検査が守る）: ref で書き換え・取り出しできる場所は、own の型の変数の場所そのものではない
     （`ref a` は a の先を指す。`ref mut own T` を変数から作る道はない。2026-10-29 に `ref mut a`・ジェネリクス・クロージャの取り込みで試し、どれも型検査で止まった）。
   - 循環は前提にしない。ムーブした own の先を ref で書き換えると持ち主の循環を作れる（`r = ref mut *a; (*r).next = Some(a)`）。
     そのとき解放されうるのは受け渡し中（`nested`）の区画だけで、事実は付かないので実行時の検査が残る（試して、検査が捕まえることを確かめた）。
3. **要素を動かす操作の一覧**: リストの中身を解放・移動しうる操作は、すべて `elems`（`kill_elems`）か `holds` の付いた上書き・解放として扱われている。
   一覧はどれも「書いたものだけ動かさない。ほかは動かしうる」の形にしてあり、新しく足した操作は何もしなければ安全側になる。
   - 組み込み: `pure_builtin`（analyze.rs と jwc/analyze.jw に同じ一覧）。名前を 1 つずつ書く（2026-10-29 まで `str.`・`int.`・`f64.` は
     名前の頭だけで決めていて、新しいメソッドが黙って「動かさない」扱いになりえた）。今の一覧は、数・真偽・文字列だけを受け取り、新しい値を返すか表示するだけ。
   - リストの操作（`SeqOp`）: len・contains・get・keys だけが動かさない。
   - 呼び出し: 引数・戻り値・取り込みが数・真偽・文字列だけの直接呼び出し（`scalar_call`。型で決まる）だけが動かさない。関数の値・interface の呼び出しは動かしうる。
   - 上書き・解放: 型で決まる（`may_hold_list` / `value_may_hold`。数・真偽・文字列・ref 以外は持ちうる）。
4. **型の決まり**（1 本ずつ確かめる。下の「検査器」）: own の型の変数は `copy` されない（表示・計算のために読むだけのものを除く）、
   ref の `x = move x` は作られない、own の変数そのものを指す ref は作られない。検査器が IR の上で確かめ、破れていれば通さない。
5. **関数の入口**（1 本ずつ確かめる）: Pinned の引数には、すべての呼び出しで生きた ref が渡る。検査器が Pinned の表を自分で計算し、
   すべての呼び出し（直接・interface）で確かめる。関数を値として使う・スレッドの入口にする・main は、Pinned の引数を持てない。
6. **範囲外**:
   - スレッド: ref をほかのスレッドへ渡せないこと（型検査の `sendable`）は型の決まりとして証明の外。検査器はスレッドの入口に Pinned がないことだけを確かめる。
   - 2 段目以降の `*` は、いつも実行時に検査する（検査器は、2 段目以降が消されていれば通さない）。
   - 値・型の中身、`LIST_SHARED` の印（何度付けても同じなので省いた）。

## 検査器（1 本ずつ確かめる。2026-10-29）

コンパイラそのもの（型検査・関数をまたぐ解析）を証明する代わりに、コンパイルした結果を 1 本ずつ、証明済みの検査器で確かめる。

- jwc が解析のあとの IR を JSON で書き出す（`jwc --ir-cert`、`jwc/ircert.jw`）。
- 検査器（`formal/JabberwockFormal/CheckerMain.lean`、`lake build elision-check`）が読み、IR を模型の操作に読み替え、
  ブロックの入口の事実と Pinned の表を自分で計算し直してから、`verifyFast` で確かめる。
- 証明: `verifyFast` が通れば `verifyFunc` も通り（`ElisionCheckFast.verifyFast_sound`）、`verifyFunc` が通った関数では、
  関数の始まりで事実が正しければ、どの実行でも、消した検査は必ず通り（`ElisionCheck.verify_elided`）、
  Pinned の引数には生きた ref が渡る（`verify_live`）。関数の始まりの事実は `start_sound` で、呼ぶ側の `verify_live` と
  OwnershipFrame の `callee_preserves_caller` から言える。
- 証明の外に残るのは、**読み替え表**（`CheckerMain.lean` の `Map` の節。IR の命令を模型のどの操作と読むか）と、
  模型の操作の動き（`Exec`。実装がそのとおり動くこと）だけ。読み替え表は analyze.rs の場合分けと 1 対 1 に書いてあり、
  分からない形は通さない。
- 実行: `python3 scripts/elision_check.py --jwc <jwc>`（結果は `target/elision-check/summary.json`。VERIFIED / NOT_VERIFIED / TOOL_ERROR）。

2026-10-29 の結果（Linux・jwc 版）: examples・tests/jwc_cases.txt・tests/safety・bench/jabberwock・jwc 自身の 346 本がすべて VERIFIED
（jwc 自身は 691 関数・消した検査 5837 か所）。IR まで行かない 292 本（型のエラーを確かめるテストなど）は数えただけ。
わざと IR の検査を全部消したもの（dangling_scope など 3 本）は NOT_VERIFIED になることを確かめた。

読み替えで analyze.rs より慎重にしたところ（検査器が通さなければ分かる。今は全部通っている）:
- `x = ref (*x)[i]` のように、自分を指す RefElem を入れない。
- `take` で変数そのものから取り出したら、その変数の事実を消す。
- own の変数に、新しい持ち主と言えない値（たどり着かない枝の `()` など）を入れたら、事実を付けない。

## 標準ライブラリに組み込みを足すとき

新しい組み込みは、模型では `elems` か `pure` のどちらか（効き方）だけで決まる。

- 何もしなければ `elems`（リストの中身を動かしうる）として扱われる。安全で、検査が少し残るだけ。
- 数・真偽・文字列だけを受け取り、何も解放しない組み込みは、`pure_builtin` の一覧（analyze.rs と jwc/analyze.jw の両方）に名前を足すと `pure` になる。
  足す前に前提 3 を守っていることを確かめる。
- Rust 版は凍結しているので、jwc にだけある組み込みは jwc の一覧にだけ足す。

新しい種類の事実や、`ref_fact` の新しい場合を足すときは、`Fact`・`transfer`・`Exec` に足して `step_sound` を直す。

## 証明の途中で見つけて直したこと

呼び出しの引数の判定（`operands`）は、読む前の事実を取っておき、すべて読み終えたあとで `is_live` を見ていた。
`is_live` は `RefSlot` を事実の表を見ずに真とするので、`f(r, y)` で r が y を借りた ref（`RefSlot(y)`）、y が後の引数でムーブされると、
取っておいた事実のせいで安全と判定されうる（`old_argument_check_is_unsound`）。今の下げ方は引数を一度一時変数に入れるので
実際のプログラムではこの形にならないが、両コンパイラで、読んだ値を名前のない変数に置いて `kill_all` が効くようにした。
