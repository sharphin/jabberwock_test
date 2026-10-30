"""formal_tool.py（形式検証の一括実行）の集計の回帰テスト。

一度だけ本物の段を全部動かして、各段のログと報告を記録する。そのあと段を「記録の再生」に差し替え、
再生の途中で報告やログを書き換えて、集計が NOT_PROVEN を PASS にしないこと・改ざんを TOOL_ERROR にすることを確かめる。
Lean（lake）が要る。なければ飛ばす（未実行であって成功ではない）。
"""
import contextlib
import io
import json
from pathlib import Path
import shutil
import tempfile
import unittest

import formal_tool as ft

REAL_RUN_STAGE = ft.run_stage


def report_path(argv):
    """段の引数の `--output` の次（報告の置き場所）"""
    return Path(argv[argv.index("--output") + 1]) if "--output" in argv else None


@unittest.skipUnless(shutil.which("lake") and shutil.which("lean"), "Lean (lake) unavailable")
class FormalToolAggregation(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="jw-formal-tool-test-")
        cls.recorded = {}

        def record(name, argv, output, timeout):
            stage = REAL_RUN_STAGE(name, argv, output, timeout)
            path = report_path(argv)
            cls.recorded[name] = {
                "stage": {k: v for k, v in stage.items() if k not in ("log_evidence", "stdout", "stderr")},
                "stdout": Path(stage["stdout"]).read_bytes(),
                "stderr": Path(stage["stderr"]).read_bytes(),
                "report": path.read_bytes() if path and path.is_file() else None,
            }
            return stage

        ft.run_stage = record
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                cls.recorded_exit = ft.main(["--output", str(Path(cls.temporary.name) / "recorded")])
        finally:
            ft.run_stage = REAL_RUN_STAGE

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def replay(self, change=None):
        """記録を再生して集計だけをもう一度動かす。change(name, output, record) で段ごとに書き換える"""
        output = Path(tempfile.mkdtemp(dir=self.temporary.name))

        def replay_stage(name, argv, out, timeout):
            saved = self.recorded[name]
            record = {"stage": dict(saved["stage"]), "report": saved["report"],
                      "stdout": saved["stdout"], "stderr": saved["stderr"]}
            if change:
                change(name, out, record)
            stdout, stderr = out / (name + ".stdout.log"), out / (name + ".stderr.log")
            stdout.write_bytes(record["stdout"])
            stderr.write_bytes(record["stderr"])
            path = report_path(argv)
            if path and record["report"] is not None:
                path.write_bytes(record["report"])
            stage = dict(record["stage"], stdout=str(stdout), stderr=str(stderr))
            return ft.attach_log_fingerprints(stage, stdout, stderr)

        ft.run_stage = replay_stage
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                code = ft.main(["--output", str(output)])
        finally:
            ft.run_stage = REAL_RUN_STAGE
        return code, json.loads((output / "summary.json").read_text(encoding="utf-8"))

    def test_recorded_run_is_an_honest_not_proven(self):
        # 正しく動いた記録の再生は、未証明を NOT_PROVEN のまま報告し、道具としては OPERATIONAL
        self.assertEqual(self.recorded_exit, 1)
        code, summary = self.replay()
        self.assertEqual(code, 1)
        self.assertEqual(summary["status"], "NOT_PROVEN")
        self.assertEqual(summary["infrastructure_status"], "OPERATIONAL")
        self.assertEqual(summary["log_integrity"], "MATCHED")

    def test_overstated_refinement_is_a_tool_error(self):
        # 段が「全部証明した」と書き換えても、ほかの段と食い違うので PROVEN にならない
        def overstate(name, out, record):
            if name == "refinement":
                report = json.loads(record["report"])
                report.update(status="PROVEN", proved=report["required"])
                record["report"] = json.dumps(report).encode("utf-8")
        code, summary = self.replay(overstate)
        self.assertEqual(code, 2)
        self.assertEqual(summary["status"], "TOOL_ERROR")

    def test_log_replaced_after_its_stage_is_invalid_evidence(self):
        def replace_log(name, out, record):
            if name == "coverage":
                (out / "inventory.stdout.log").write_bytes(b"replaced\n")
        code, summary = self.replay(replace_log)
        self.assertEqual(code, 2)
        self.assertEqual(summary["log_integrity"], "INVALID_EVIDENCE")

    def test_report_changed_after_inspection_is_invalid_evidence(self):
        def rewrite_report(name, out, record):
            if name == "coverage":
                path = out / "inventory.json"
                path.write_bytes(path.read_bytes() + b" ")
        code, summary = self.replay(rewrite_report)
        self.assertEqual(code, 2)
        self.assertTrue(any("report changed after inspection" in f for f in summary["log_integrity_failures"]))

    def test_missing_report_is_not_operational(self):
        def drop(name, out, record):
            if name == "semantic-composition":
                record["report"] = None
        code, summary = self.replay(drop)
        self.assertEqual(code, 2)
        self.assertEqual(summary["infrastructure_status"], "INVALID")

    def test_timed_out_stage_is_a_tool_error(self):
        def timeout(name, out, record):
            if name == "inventory":
                record["stage"] = {"name": name, "status": "TIMEOUT", "seconds": 1.0}
        code, summary = self.replay(timeout)
        self.assertEqual(code, 2)
        self.assertEqual(summary["status"], "TOOL_ERROR")


if __name__ == "__main__":
    unittest.main()
