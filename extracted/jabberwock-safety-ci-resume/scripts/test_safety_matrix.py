"""Regression cases for compiler attribution and separate goal/proof paths."""
import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import safety_report as report
import safety_compilers as matrix

class MatrixTests(unittest.TestCase):
    def test_catalog_is_compiler_specific_and_no_formal_tool(self):
        for target in ("windows", "linux"):
            preparations, probes = matrix.catalog(target, 1, Path("work"))
            self.assertEqual(probes[0]["compiler"], "jwc")
            self.assertFalse(any(p["lake"] or "scripts/check_formal.py" in p["cmd"] for p in preparations + probes))
            for compiler in matrix.COMPILERS:
                own = [p for p in probes if p["compiler"] == compiler]
                self.assertEqual({n for p in own for n in p["items"]}, set(range(1, 13)))
                self.assertEqual(len([p for p in own if p.get("goal")]), 5)
                self.assertTrue(all(p.get("expected_subject") == compiler for p in own if p.get("case_report")))

    def test_wrong_subject_or_missing_case_is_infrastructure_error(self):
        with tempfile.TemporaryDirectory() as d:
            output = Path(d) / "cases.json"
            for subject, cases in [("rust", [{"id": "a", "status": "pass"}]),
                                   ("jwc", []), ("jwc", [{"id": "a", "status": "pass"}] * 2)]:
                p = report.safety.probe("contracts", [1], [sys.executable])
                p.update(compiler="jwc", expected_subject="jwc", case_report=str(output), required_case_ids=["a"])
                def execute(*a, **kw):
                    output.write_text(json.dumps({"subject": subject, "results": cases}))
                    return subprocess.CompletedProcess(a[0], 0, "", "")
                with mock.patch.object(report, "execute_probe", side_effect=execute), contextlib.redirect_stdout(io.StringIO()):
                    result = report.run_probe(p, Path(d), False)
                self.assertEqual(result["status"], "INFRA_ERROR")

    def test_rust_failure_cannot_become_jwc_failure(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "report.json"
            def execute(p, *_):
                return {"name": p["name"], "status": "FAIL" if p["name"] == "rust-contracts" else "PROBE_PASS"}
            with mock.patch.object(report.suites, "host_os", return_value="linux"), \
                 mock.patch.object(report, "version", return_value="mock"), \
                 mock.patch.object(report, "run_probe", side_effect=execute), contextlib.redirect_stdout(io.StringIO()):
                code = report.main(["--output", str(path)])
            data = json.loads(path.read_text(encoding="utf-8"))
            self.assertEqual(code, 1)
            self.assertEqual(len(data["items"]), 24)
            self.assertEqual(len(data["goals"]), 10)
            self.assertTrue(all(i["status"] == "PROBES_PASS_ONLY" for i in data["items"] if i["compiler"] == "jwc"))
            self.assertTrue(all(i["acceptance"] == "BLOCKED" for i in data["items"]))

    def test_unknown_status_cannot_exit_successfully(self):
        with tempfile.TemporaryDirectory() as d:
            output = Path(d) / "matrix.json"
            def fake_probe(p, *_):
                return {"name": p["name"],
                        "status": "UNRECOGNIZED" if p["name"] == "jwc-contracts" else "PROBE_PASS"}
            with mock.patch.object(report.suites, "host_os", return_value="linux"), \
                 mock.patch.object(report, "version", return_value="mock"), \
                 mock.patch.object(report, "run_probe", side_effect=fake_probe), \
                 contextlib.redirect_stdout(io.StringIO()):
                code = report.main(["--output", str(output)])
            self.assertEqual(code, 1)
            data = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(len(data["items"]), 24)
            self.assertTrue(any(x["status"] == "NOT_RUN" for x in data["items"] if x["compiler"] == "jwc"))

    def test_legacy_effects_duplicate_configuration_is_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            output = Path(d) / "effects.json"
            probe = report.safety.probe("effects", [10], [sys.executable])
            probe.update(compiler="jwc", expected_subject="jwc", case_report=str(output))
            def execute(*args, **kw):
                cases = [{"case": "effect.jw", "configuration": "jwc-native-O1",
                          "compiler": "jwc", "status": "pass"}] * 2
                output.write_text(json.dumps({"subject": "jwc", "results": cases}))
                return subprocess.CompletedProcess(args[0], 0, "", "")
            with mock.patch.object(report, "execute_probe", side_effect=execute), contextlib.redirect_stdout(io.StringIO()):
                verdict = report.run_probe(probe, Path(d), False)
            self.assertEqual(verdict["status"], "INFRA_ERROR")

    def test_missing_per_case_compiler_is_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            output = Path(d) / "cases.json"
            probe = report.safety.probe("contracts", [1], [sys.executable])
            probe.update(compiler="jwc", expected_subject="jwc",
                         case_report=str(output), required_case_ids=["a"])
            def execute(*a, **kw):
                output.write_text(json.dumps({
                    "subject": "jwc", "results": [{"id": "a", "status": "pass"}]}))
                return subprocess.CompletedProcess(a[0], 0, "", "")
            with mock.patch.object(report, "execute_probe", side_effect=execute), contextlib.redirect_stdout(io.StringIO()):
                verdict = report.run_probe(probe, Path(d), False)
            self.assertEqual(verdict["status"], "INFRA_ERROR")

    def test_dynamic_inventory_rejects_omission_and_duplicates(self):
        with tempfile.TemporaryDirectory() as d:
            output = Path(d) / "cases.json"
            p = report.safety.probe("allocation", [8], [sys.executable])
            p.update(compiler="jwc", expected_subject="jwc", case_report=str(output))
            for expected, actual in [
                (["a", "b"], ["a"]),
                (["a", "a"], ["a"]),
                (["a"], ["a", "a"]),
                ([], []),
            ]:
                def execute(*args, **kwargs):
                    output.write_text(json.dumps({
                        "subject": "jwc", "expected_case_ids": expected,
                        "results": [{"id": x, "compiler": "jwc", "status": "pass"} for x in actual]
                    }))
                    return subprocess.CompletedProcess(args[0], 0, "", "")
                with mock.patch.object(report, "execute_probe", side_effect=execute), contextlib.redirect_stdout(io.StringIO()):
                    result = report.run_probe(p, Path(d), False)
                self.assertEqual(result["status"], "INFRA_ERROR")

    def test_zero_tests_cannot_pass_detector(self):
        p = report.safety.probe("negative", [2], [sys.executable])
        p["required_test_names"] = ["detector"]
        with tempfile.TemporaryDirectory() as d, \
             mock.patch.object(report, "execute_probe", return_value=subprocess.CompletedProcess([], 0, "running 0 tests", "")), \
             contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(report.run_probe(p, Path(d), False)["status"], "INFRA_ERROR")

if __name__ == "__main__":
    unittest.main()
