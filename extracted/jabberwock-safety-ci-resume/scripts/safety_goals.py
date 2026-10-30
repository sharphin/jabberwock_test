"""Practical evidence for five goals. Does not import or execute proof tooling.

（01 の jwc-elision-certificate だけは Lean で作った検査器を動かすが、定理の検査ではなく、プログラムごとの IR の確かめ直し）"""
GOALS = {
    "semantics": ("コンパイラの意味保存", [1,5,7,9,10,12]),
    "memory": ("メモリ安全性", [2,3,4,6,8,11]),
    "types": ("型安全性", [2,9,10]),
    "ownership": ("所有権・参照の整合性", [1,2,3,8,11]),
    "checks": ("実行時チェックの正しさ", [1,2,4,5,8,9]),
}
# Outstanding coverage obligations are explicit; passing finite probes cannot erase them.
GAPS = {
    1: ["全IR命令・全制御フローでの検査省略条件の網羅"],
    2: ["公開言語機能の型の通る攻撃ケースの網羅"],
    3: ["競合・同期・回収の全共有経路とスケジュールの検査"],
    4: ["LLVM poison/undef・属性・alias規則の専用検証（UBSanだけでは不足）"],
    5: ["全サポート対象の最適化・リンク構成の網羅"],
    6: ["未初期化値の全生成経路の検査"],
    7: ["stage2/stage3への12項目・5目標の全検査の適用"],
    8: ["コンパイラ内部確保・スタック・スレッド・OS資源の失敗注入"],
    9: ["入れ子型・全レイアウト演算の境界値の網羅"],
    10: ["仕様の全機能と正負ケースの対応表の完成"],
    11: ["全所有型・例外的終了経路・FFI境界の回収検査"],
    12: ["実機・CPU・ツールチェーンのサポート構成表の全実行"],
}

def goals(items, compilers, target, catalog, produced):
    result = []
    for compiler in compilers:
        own = {i["item"]: i for i in items if i["compiler"] == compiler}
        for key, (name, numbers) in GOALS.items():
            evidence = [own[n] for n in numbers]
            direct_names = [p["name"] for p in catalog if p.get("compiler") == compiler and p.get("goal") == key]
            direct = [produced.get(n, {"status": "NOT_RUN"}) for n in direct_names]
            statuses = {i["status"] for i in evidence}
            statuses.update("FAIL" if r["status"] in ("FAIL", "CRASH", "TIMEOUT", "INFRA_ERROR", "INTERRUPTED")
                            else "NOT_RUN" if r["status"] != "PROBE_PASS" else "PROBES_PASS_ONLY" for r in direct)
            if not direct:
                statuses.add("NOT_RUN")
            result.append({"id": target + "-" + compiler + "-" + key, "compiler": compiler,
                           "name": name, "direct_probe_names": direct_names, "kind": "practical-tests", "item_ids": [i["id"] for i in evidence],
                           "probe_names": sorted({p for i in evidence for p in i["probe_names"]}),
                           "status": "FAIL" if "FAIL" in statuses else "NOT_RUN" if statuses & {"NOT_RUN", "PLANNED"} else "PROBES_PASS_ONLY",
                           "acceptance": "BLOCKED", "coverage_gaps": sorted({g for i in evidence for g in i["coverage_gaps"]})})
    return result


# 合意した基準（2026-10-30、オーナーと合意）。GAPS は消さずに残し、その外側に「終わりのある合格条件」を足す。
# 条件を満たしても acceptance（34 の義務の全体の受け入れ）は BLOCKED のまま。結果は別の欄 agreed に書く。
# required: この OS・このコンパイラで、名前（"<コンパイラ>-" を除いた部分）の probe が全部 PROBE_PASS であること。
# 必要な probe が catalog にない（例: Rust の 01 は検査器が jwc の IR しか読まない）なら満たさない。
# 12 はこのレポートの OS の分だけ。12 全体の合格は Linux と Windows の両方のレポートで MET のとき（混ぜない）。
AGREED = {
    1: ("消した検査が全部 Lean の検査器で VERIFIED（jwc 自身を含む全コーパス）。fuzz -O1/-O2 が成功",
        {"linux": ["elision-certificate", "fuzz-O1", "fuzz-O2"], "windows": ["elision-certificate", "fuzz-O1", "fuzz-O2"]}),
    2: ("tests/safety の攻撃ケースと契約の試験が全部成功",
        {"linux": ["contracts", "address", "retirement"], "windows": ["contracts", "address", "retirement"]}),
    3: ("データ競合の検出器の試験が成功（Linux は検出できることの確認も）",
        {"linux": ["thread", "thread-negative"], "windows": ["thread"]}),
    4: ("未定義動作の検出器と LLVM の検証器が成功（Linux は検出できることの確認も）",
        {"linux": ["undefined", "llvm-verifier", "undefined-negative"], "windows": ["undefined", "llvm-verifier"]}),
    5: ("対応する最適化は -O1 と -O2。両方で成功",
        {"linux": ["fuzz-O1", "fuzz-O2", "address"], "windows": ["fuzz-O1", "fuzz-O2", "address"]}),
    6: ("未初期化メモリの検出器の試験が成功（Linux は検出できることの確認も）",
        {"linux": ["memory", "memory-negative"], "windows": ["memory"]}),
    7: ("jwc: stage1→2→3 が作れ、LLVM の出力が一致し、stage2/3 の出したコードが動く。Rust: jwc を最初に作る役の試験が成功",
        {"linux": ["bootstrap", "retirement"], "windows": ["bootstrap", "retirement"]}),
    8: ("実行時の確保の失敗と資源の上限で決めたとおり止まる。コンパイラ内部の失敗注入は範囲外",
        {"linux": ["allocation", "resources"], "windows": ["allocation"]}),
    9: ("配列の大きさの境界の試験が成功",
        {"linux": ["array-limits"], "windows": ["array-limits"]}),
    10: ("spec.md の機能ごとに正しい例と拒否される例が少なくとも 1 つずつある対応表があり、全部成功",
         {"linux": ["contracts", "effects", "spec-coverage"], "windows": ["contracts", "effects", "spec-coverage"]}),
    11: ("回収・退役・途中で止まったときの試験が成功。言語に FFI がないので FFI は範囲外",
         {"linux": ["allocation", "address", "runtime-retirement", "effects"],
          "windows": ["allocation", "address", "runtime-retirement", "effects"]}),
    12: ("対応する構成は Linux x86_64 と Windows（オーナーの実機）。このレポートの OS で成功",
         {"linux": ["bootstrap", "effects"], "windows": ["bootstrap", "effects"]}),
}
# Rust 版は自己ホストしないので、07・12 は jwc を最初に作る役の試験（bootstrap-source）と回帰の試験を見る
RUST_REQUIRED = {7: ["bootstrap-source"], 12: ["effects", "source-regression"]}


def agreed(item, compiler, target, produced, references):
    """合意した基準を満たすか。item は safety_report が作った項目の dict"""
    condition, required = AGREED[item["item"]]
    suffixes = RUST_REQUIRED.get(item["item"], required[target]) if compiler == "rust" else required[target]
    names = [compiler + "-" + s for s in suffixes]
    absent = [n for n in names if n not in references]
    failing = [n for n in names if n in references and produced.get(n, {}).get("status") != "PROBE_PASS"]
    met = item["status"] == "PROBES_PASS_ONLY" and not absent and not failing
    return {"status": "MET" if met else "NOT_MET", "condition": condition, "required_probes": names,
            "absent_probes": absent, "not_passed_probes": failing}


def agreed_goal(goal, items):
    """目標は、関係する項目が全部 MET で、目標そのものが PROBES_PASS_ONLY のとき MET"""
    own = {i["item"]: i for i in items if i["compiler"] == goal["compiler"]}
    _, numbers = GOALS[goal["id"].rsplit("-", 1)[1]]
    unmet = [n for n in numbers if own[n]["agreed"]["status"] != "MET"]
    met = goal["status"] == "PROBES_PASS_ONLY" and not unmet
    return {"status": "MET" if met else "NOT_MET", "unmet_items": unmet}
