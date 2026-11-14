# F2-001: 有限世代・退役・参照の部分モデル

様式は [形式証明専用の検証様式](../../docs/formal-verification.md)。

## 識別と範囲

- 主分類F2。関連F4/F5、G2/G4/G5、M3/O4/C1。
- 仕様: [形式仕様の有限世代と退役](../../docs/formal-semantics.md#有限世代と退役の規則)。
- ソース: [Generation.lean](../JabberwockFormal/Generation.lean)、名前空間`JabberwockFormal.Generation`。
- 対象: 自然数の世代、live/vacant/retired、許された遷移と任意の有限遷移列、抽象的な読書き判定。
- 対象外: heap全体、keyの割当て、payload、path、機械アドレス、Cのbit配置、並行実行、検査省略。
  確保成功や任意のプログラムの停止性は主張しない。

## 命題と前提

中心となる命題は`JabberwockFormal.Generation.no_resurrection`。

```text
量化: limit, epoch : Nat、before, after, final : Cell
前提: Valid epoch before
      Invalidates before after
      Steps limit after final
結論: ¬ Valid epoch final
```

有効だった参照を無効化した後、許された有限の継続では、その参照は再び有効にならない。
実際に発行され、初めに有効だったrefが対象。将来の世代を勝手に持つ不正なref全般についての主張ではない。
前提を満たす例は、世代0のliveを借り、世代0のretiredへ移し、その状態に留まる遷移列。
具体実装がこの遷移関係だけに従うことは、この命題の証明からは導かれない。

同じ記録はモジュール内の全16定理を扱う。上限の保存には`WellFormed`の前提を用い、世代の単調性、退役状態の維持、
無効化の選択可能性、読書きの許可/拒否も検査する。全定理名と実際の公理依存は [検査記録](2026-10-26-model-check.json)。
`wrapping_revives_at_two`は旧周回規則の小さい反例であり、現行runtimeへの攻撃成功や新たな安全性保証1件とは扱わない。

## 証明と実装の状態

| 欄 | 状態 |
|---|---|
| 証明の状態 | モデル内証明済み |
| 実装との接続 | 未証明 |
| 対応を検討する実装 | `src/interp.rs`、`runtime/jw_rt.c`、Rust/jwcのLLVM生成。退役実装の記録は [こちら](../../docs/generation-retirement.md) |
| 具体状態との対応定理 | なし。Slot/ヘッダー/保持表とモデルの対応を今後定義する |
| 信頼する部分 | Lean 4.24.0のkernel・標準ライブラリと固定ツールチェーン、検査scriptの動作 |
| 残るレビュー | モデルを全言語へ拡張した際の妥当性、実装の全遷移との対応。現在の部分モデルを全言語の仕様として扱わない |

許可する標準公理は`propext`・`Quot.sound`・`Classical.choice`。実際の依存は定理ごとに監査して記録している。
Rust/C/LLVMがこのモデルに従うという仮定を追加し、その仮定を実装の証明と称することはしない。

## 機械検査と維持

2026-10-26、ローカルLinuxで`python3 scripts/check_formal.py`が終了0。
全22定理のうち、このモジュールの16定理のビルド・公理監査が成功した。
検査対象の版は`d11e1ac0def2f7cf4ca6db1d9c6845aef1d30216`。入力ハッシュ・日時・実出力は [検査記録](2026-10-26-model-check.json) に固定する。
この版は機械検査したモデルの識別であり、同じcommitのコンパイラ全体を証明したという意味ではない。

次はpayload・key・pathを含む評価状態と、具体的な表現/操作の対応へ接続する。
仕様、遷移、参照の有効性、定理、依存、公理方針、検査器が変われば再検査・再確認する。
実装変更時には接続すべき範囲を更新する。記録の成功を最新の実装保証へ自動的に持ち越さない。
