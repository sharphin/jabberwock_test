"""Practical evidence for five goals. Does not import or execute proof tooling."""
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
