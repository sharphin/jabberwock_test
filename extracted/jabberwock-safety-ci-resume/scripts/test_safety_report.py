#!/usr/bin/env python3
"""Regression tests for the single-file safety report runner (no compiler needed)."""
import json
import os
import contextlib
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import safety_report as report


class SafetyReportTests(unittest.TestCase):
    def test_locked_destination_keeps_complete_alternate_and_original(self):
        with tempfile.TemporaryDirectory() as d:
            target = Path(d) / "report.json"
            target.write_text('OLD', encoding="utf-8")
            writer = report.ReportWriter(target)
            with mock.patch.object(Path, "replace", side_effect=PermissionError("locked")) as replace, \
                 mock.patch.object(report.time, "sleep"), contextlib.redirect_stdout(io.StringIO()):
                writer.save({"run_status": "RUNNING", "results": ["saved"]})
            self.assertEqual(replace.call_count, 5)
            self.assertEqual(target.read_text(), "OLD")
            self.assertNotEqual(writer.path, target)
            self.assertEqual(json.loads(writer.path.read_text())["results"], ["saved"])
            alternate = writer.path
            writer.save({"run_status": "COMPLETE"})
            self.assertEqual(writer.path, alternate)
            self.assertEqual(json.loads(alternate.read_text())["run_status"], "COMPLETE")

    def test_transient_lock_retries_without_changing_destination(self):
        with tempfile.TemporaryDirectory() as d:
            target = Path(d) / "report.json"
            writer = report.ReportWriter(target)
            original_replace = Path.replace
            attempts = []
            def replace(source, destination):
                attempts.append(source)
                if len(attempts) < 3:
                    raise PermissionError("temporary lock")
                return original_replace(source, destination)
            with mock.patch.object(Path, "replace", new=replace), mock.patch.object(report.time, "sleep"):
                writer.save({"saved": True})
            self.assertEqual(len(attempts), 3)
            self.assertEqual(writer.path, target)
            self.assertEqual(json.loads(target.read_text()), {"saved": True})
            self.assertEqual(list(Path(d).iterdir()), [target])

    def test_unwritable_report_stops_before_probes_without_repeating_save(self):
        with tempfile.TemporaryDirectory() as d:
            with mock.patch.object(report.suites, "host_os", return_value="windows"), \
                 mock.patch.object(report, "version", return_value="mock-version"), \
                 mock.patch.object(report.ReportWriter, "save", side_effect=report.ReportSaveError("disk unavailable")) as save, \
                 mock.patch.object(report, "run_probe") as probe, \
                 contextlib.redirect_stderr(io.StringIO()) as diagnostic:
                code = report.main(["--output", str(Path(d) / "report.json")], target_os="windows")
            self.assertEqual(code, 2)
            save.assert_called_once()
            probe.assert_not_called()
            self.assertIn("INFRA_ERROR", diagnostic.getvalue())

    def test_excerpt_preserves_both_ends(self):
        raw = "FIRST ERROR\n" + "x" * 50000 + "\nFINAL SUMMARY"
        trimmed = report.excerpt(raw)
        self.assertTrue(trimmed["truncated"])
        self.assertIn("FIRST ERROR", trimmed["text"])
        self.assertIn("FINAL SUMMARY", trimmed["text"])
        self.assertEqual(trimmed["sha256"], report.hashlib.sha256(raw.encode()).hexdigest())

    def test_success_only_means_probe_pass(self):
        with tempfile.TemporaryDirectory() as d:
            probe = report.safety.probe("mock-probe", [1], [sys.executable, "-c", "print('OK')"])
            result = report.run_probe(probe, Path(d), False)
            self.assertEqual(result["status"], "PROBE_PASS")
            self.assertEqual(result["actual"]["exit_code"], 0)
            self.assertIn("OK", result["actual"]["stdout"]["text"])

    def test_failure_includes_diagnostic_and_exit(self):
        with tempfile.TemporaryDirectory() as d:
            p = report.safety.probe("mock-probe", [8],
                                    [sys.executable, "-c", "import sys; sys.stderr.write('error: test\\n'); sys.exit(1)"])
            result = report.run_probe(p, Path(d), False)
            self.assertEqual(result["status"], "FAIL")
            self.assertEqual(result["actual"]["exit_code"], 1)
            self.assertIn("error: test", result["actual"]["stderr"]["text"])

    def test_nested_case_failure_is_not_green(self):
        with tempfile.TemporaryDirectory() as d:
            location = Path(d)
            def produce(*args, **kwargs):
                (location / "edges-8-9.json").write_text(json.dumps(
                    {"results": [{"item": 8, "case": "resource-allocation", "case_status": "fail",
                                  "observed": {"exit": 2, "stderr": "internal error"}}]}), encoding="utf-8")
                return subprocess.CompletedProcess(args[0], 0, "", "")
            p = report.safety.probe("edges-8-9", [8, 9], [sys.executable, "-c", "pass"])
            with mock.patch.object(report, "execute_probe", side_effect=produce):
                result = report.run_probe(p, location, False)
            self.assertEqual(result["status"], "FAIL")
            self.assertEqual(result["case_report"]["results"][0]["observed"]["exit"], 2)

    def test_unavailable_is_not_passed(self):
        with tempfile.TemporaryDirectory() as d:
            p = report.safety.probe("absent-test", [12], ["__nonexistent_safety_executable__"])
            result = report.run_probe(p, Path(d), False)
            self.assertEqual(result["status"], "NOT_RUN")

    def test_retirement_without_clang_is_not_run(self):
        with tempfile.TemporaryDirectory() as d:
            p = next(p for p in report.safety.catalog(1, Path(d)) if p["name"] == "retirement")
            with mock.patch.object(report.shutil, "which", return_value=None), \
                 mock.patch.object(report, "execute_probe") as run:
                result = report.run_probe(p, Path(d), False)
            self.assertEqual(result["status"], "NOT_RUN")
            self.assertEqual(result["reason"], "clang unavailable")
            run.assert_not_called()

    def test_native_retirement_requires_execution_and_does_not_regenerate_sources(self):
        with tempfile.TemporaryDirectory() as d:
            p = next(p for p in report.safety.catalog(1, Path(d)) if p["name"] == "retirement")
            completed = subprocess.CompletedProcess(p["cmd"], 0, "", "")
            with mock.patch.object(report.shutil, "which", return_value="/tools/clang"), \
                 mock.patch.dict(os.environ, {"JWC_REGEN": "1"}), \
                 mock.patch.object(report, "execute_probe", return_value=completed) as run:
                result = report.run_probe(p, Path(d), False)
            self.assertEqual(result["status"], "PROBE_PASS")
            child_env = run.call_args.kwargs["env"]
            self.assertEqual(child_env["JW_REQUIRE_RETIREMENT_TESTS"], "1")
            self.assertNotIn("JWC_REGEN", child_env)

    def test_rust_profiles_run_without_clang_but_disclose_missing_native_coverage(self):
        with tempfile.TemporaryDirectory() as d:
            for p in report.safety.catalog(1, Path(d)):
                if p["name"] not in ("cargo-test", "release-test"):
                    continue
                completed = subprocess.CompletedProcess(p["cmd"], 0, "", "")
                def which(name):
                    return None if name == "clang" else "/tools/" + name
                with mock.patch.dict(os.environ, {"JW_CLANG": "clang"}), \
                     mock.patch.object(report.shutil, "which", side_effect=which), \
                     mock.patch.object(report, "execute_probe", return_value=completed) as run:
                    result = report.run_probe(p, Path(d), False)
                self.assertEqual(result["status"], "PROBE_PASS")
                self.assertIn("coverage_notes", result)
                self.assertIn("native/jwc/sanitizer", result["coverage_notes"][0])
                run.assert_called_once()

    def test_stale_case_report_is_not_reused(self):
        with tempfile.TemporaryDirectory() as d:
            location = Path(d)
            old = location / "edges-8-9.json"
            old.write_text(json.dumps({"results": [{"item": 8, "case_status": "pass"}]}))
            p = report.safety.probe("edges-8-9", [8, 9], [sys.executable, "-c", "pass"])
            with mock.patch.object(report, "execute_probe", return_value=subprocess.CompletedProcess(p["cmd"], 0, "", "")):
                result = report.run_probe(p, location, False)
            self.assertEqual(result["status"], "INFRA_ERROR")
            self.assertNotIn("case_report", result)
            self.assertFalse(old.exists())

    def test_incomplete_detailed_report_is_not_passed(self):
        with tempfile.TemporaryDirectory() as d:
            location = Path(d)
            destination = location / "array-limits.json"
            p = report.suites.detailed("array-limits", [9], [sys.executable, "-c", "pass"], destination, 16)
            def produce(*args, **kwargs):
                destination.write_text(json.dumps({"results": [{"item": 9, "case": "array-limit", "case_status": "pass"}]}))
                return subprocess.CompletedProcess(args[0], 0, "", "")
            with mock.patch.object(report, "execute_probe", side_effect=produce):
                result = report.run_probe(p, location, False)
            self.assertEqual(result["status"], "INFRA_ERROR")
            self.assertIn("expected 16 cases", result["reason"])

    def test_cannot_label_linux_execution_as_windows(self):
        with mock.patch.object(report.suites, "host_os", return_value="linux"), \
             mock.patch.object(report, "run_probe") as run, \
             contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            report.main([], target_os="windows")
        self.assertEqual(error.exception.code, 2)
        run.assert_not_called()

    def test_each_os_has_its_own_twelve_items_and_default_output(self):
        def fake_probe(p, work, dry):
            return {"name": p["name"], "items": p["items"], "target_os": p["target_os"],
                    "status": "NOT_RUN" if p.get("unavailable_reason") else "PROBE_PASS"}
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            for target in ("windows", "linux"):
                with mock.patch.object(report, "ROOT", root), \
                     mock.patch.object(report.safety_compilers.detectors, "executable", return_value=None), \
                     mock.patch.object(report.suites, "host_os", return_value=target), \
                     mock.patch.object(report, "version", return_value="recorded-version"), \
                     mock.patch.object(report, "run_probe", side_effect=fake_probe), \
                     contextlib.redirect_stdout(io.StringIO()):
                    code = report.main([], target_os=target)
                data = json.loads((root / "target" / ("safety-" + target + "-report.json")).read_text(encoding="utf-8"))
                self.assertEqual(code, 1)  # missing detectors stay NOT_RUN on both OSes
                self.assertEqual(data["suite"]["id"], target + "-compiler-matrix")
                self.assertEqual([i["item"] for i in data["items"]], list(range(1, 13)) * 2)
                self.assertEqual(len({i["id"] for i in data["items"]}), 24)
                self.assertTrue(all(i["target_os"] == target and i["id"].startswith(target + "-") for i in data["items"]))
                self.assertTrue(all(i["acceptance"] == "BLOCKED" for i in data["items"]))
                if target == "windows":
                    self.assertTrue(all(data["items"][i - 1]["status"] == "NOT_RUN" for i in (3, 4, 6)))

    def test_linux_array_failure_does_not_fail_resource_item(self):
        def fake_probe(p, work, dry):
            return {"name": p["name"], "items": p["items"],
                    "status": "FAIL" if p["name"] == "jwc-array-limits" else "PROBE_PASS"}
        with tempfile.TemporaryDirectory() as d:
            output = Path(d) / "report.json"
            with mock.patch.object(report.suites, "host_os", return_value="linux"), \
                 mock.patch.object(report, "version", return_value="recorded-version"), \
                 mock.patch.object(report, "run_probe", side_effect=fake_probe), \
                 contextlib.redirect_stdout(io.StringIO()):
                report.main(["--output", str(output)], target_os="linux")
            data = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(data["items"][7]["status"], "PROBES_PASS_ONLY")
            self.assertEqual(data["items"][8]["status"], "FAIL")

    def test_windows_plan_has_no_linux_only_commands(self):
        probes = report.suites.catalog("windows", 1, Path("work"))
        self.assertFalse(any(p["linux"] for p in probes))
        self.assertEqual(set(n for p in probes for n in p["items"]), set(range(1, 13)))
        self.assertEqual(set(n for p in probes if p.get("unavailable_reason") for n in p["items"]), {3, 4, 6, 8})
        arrays = next(p for p in probes if p["name"] == "array-limits")
        self.assertEqual(arrays["cmd"][arrays["cmd"].index("--items") + 1], "9")
        self.assertTrue(arrays["jwc_executable"].endswith(".exe"))

    def test_failure_is_saved_before_next_probe_and_interrupt_keeps_twelve_items(self):
        with tempfile.TemporaryDirectory() as d:
            output = Path(d) / "report.json"
            def probe(p, work, dry):
                if p["name"] == "rust-goal-semantics":
                    partial = json.loads(output.read_text(encoding="utf-8"))
                    self.assertEqual(partial["active_probe"], "rust-goal-semantics")
                    self.assertEqual(partial["run_status"], "RUNNING")
                    self.assertEqual(len(partial["items"]), 24)
                    failure = next(r for r in partial["results"] if r["name"] == "jwc-build")
                    self.assertEqual(failure["actual"]["stderr"]["text"], "link failed")
                    self.assertEqual(partial["items"][0]["status"], "NOT_RUN")
                    raise KeyboardInterrupt
                return {"name": p["name"], "status": "FAIL" if p["name"] == "jwc-build" else "PROBE_PASS",
                        "actual": {"stderr": {"text": "link failed"}}}
            with mock.patch.object(report.suites, "host_os", return_value="windows"), \
                 mock.patch.object(report, "version", return_value="mock-version"), \
                 mock.patch.object(report, "run_probe", side_effect=probe), \
                 contextlib.redirect_stdout(io.StringIO()):
                code = report.main(["--output", str(output)], target_os="windows")
            final = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(code, 130)
            self.assertEqual(final["run_status"], "INTERRUPTED")
            self.assertEqual(len(final["items"]), 24)
            self.assertTrue(final["items"][12]["pending_probe_names"])

    def test_failed_jwc_build_blocks_consumers_even_if_old_binary_exists(self):
        called = []
        def probe(p, work, dry):
            called.append(p["name"])
            self.assertFalse(p["jwc"])
            return {"name": p["name"], "status": "FAIL" if p["name"] == "jwc-build" else "PROBE_PASS"}
        with tempfile.TemporaryDirectory() as d:
            output = Path(d) / "report.json"
            real_is_file = Path.is_file
            def is_file_with_stale_binary(path):
                if path.name.lower() in {"jwc", "jwc.exe", "jwc-local", "jwc-local.exe",
                                         "jwc-benchmark", "jwc-benchmark.exe", "jwc-bin", "jwc-bin.exe"}:
                    return True
                return real_is_file(path)
            with mock.patch.object(report.suites, "host_os", return_value="windows"), \
                 mock.patch.object(report, "version", return_value="mock-version"), \
                 mock.patch.object(report, "run_probe", side_effect=probe), \
                 mock.patch.object(Path, "is_file", new=is_file_with_stale_binary), \
                 contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(report.main(["--output", str(output)], target_os="windows"), 1)
            data = json.loads(output.read_text(encoding="utf-8"))
            arrays = next(r for r in data["results"] if r["name"] == "jwc-array-limits")
            self.assertEqual(arrays["status"], "NOT_RUN")
            self.assertIn("jwc-build", arrays["reason"])
            self.assertIn("rust-source-regression", called)

    def test_heartbeat_and_file_output_without_capture_pipes(self):
        with tempfile.TemporaryDirectory() as d:
            paths = {s: Path(d) / s for s in ("stdout", "stderr")}
            process = mock.Mock(pid=123, returncode=0)
            process.wait.side_effect = [subprocess.TimeoutExpired(["mock"], 15), 0]
            def launch(*args, **kwargs):
                kwargs["stdout"].write(b"partial output\n")
                return process
            console = io.StringIO()
            with mock.patch.object(report.subprocess, "Popen", side_effect=launch) as popen, \
                 mock.patch.object(report.time, "monotonic", side_effect=[0, 0, 15, 15]), \
                 contextlib.redirect_stdout(console):
                result = report.execute_probe(["mock"], env={}, timeout=30, name="example", log_paths=paths)
            self.assertIn("RUNNING example: 15s", console.getvalue())
            self.assertEqual(result.stdout, "partial output\n")
            self.assertEqual(popen.call_args.kwargs["stdin"], subprocess.DEVNULL)

    def test_timeout_cleans_up_and_leaves_diagnostics_on_disk(self):
        with tempfile.TemporaryDirectory() as d:
            paths = {s: Path(d) / s for s in ("stdout", "stderr")}
            process = mock.Mock(pid=123)
            process.wait.side_effect = subprocess.TimeoutExpired(["mock"], 1)
            def launch(*args, **kwargs):
                kwargs["stderr"].write(b"before timeout\n")
                return process
            with mock.patch.object(report.subprocess, "Popen", side_effect=launch), \
                 mock.patch.object(report, "stop_process") as stop, \
                 mock.patch.object(report.time, "monotonic", side_effect=[0, 0, 1, 2]), \
                 contextlib.redirect_stdout(io.StringIO()), self.assertRaises(subprocess.TimeoutExpired):
                report.execute_probe(["mock"], env={}, timeout=1, name="example", log_paths=paths)
            stop.assert_called_once_with(process)
            self.assertEqual(paths["stderr"].read_text(), "before timeout\n")

    def test_interrupt_preserves_probe_diagnostics(self):
        with tempfile.TemporaryDirectory() as d:
            def execute(*args, **kwargs):
                kwargs["log_paths"]["stderr"].write_text("before interrupt", encoding="utf-8")
                raise KeyboardInterrupt
            p = report.safety.probe("interrupt", [1], [sys.executable, "-c", "pass"])
            with mock.patch.object(report, "execute_probe", side_effect=execute), \
                 contextlib.redirect_stdout(io.StringIO()):
                result = report.run_probe(p, Path(d), False)
            self.assertEqual(result["status"], "INTERRUPTED")
            self.assertEqual(result["actual"]["stderr"]["text"], "before interrupt")


if __name__ == "__main__":
    unittest.main()
