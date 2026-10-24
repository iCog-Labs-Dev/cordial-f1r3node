"""Offline runner regression checks. Cargo and benchmark workloads are mocked."""
import contextlib
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, MagicMock, call, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import benchmark


class DependencyProvenance(unittest.TestCase):
    def test_records_the_current_host_checkout_not_the_old_sibling(self):
        with patch.object(benchmark, "capture", return_value={"exit_code": 0, "output": ""}) as capture:
            result = benchmark.provenance()
        self.assertEqual(benchmark.HOST_DEPENDENCY, benchmark.ROOT.parent / "f1r3node-rust")
        self.assertEqual(result["host_dependency_path"], str(benchmark.HOST_DEPENDENCY))
        capture.assert_any_call(["git", "rev-parse", "HEAD"], benchmark.HOST_DEPENDENCY)
        capture.assert_any_call(["git", "status", "--porcelain"], benchmark.HOST_DEPENDENCY)
        self.assertFalse(any(call.args[-1] == benchmark.ROOT.parent / "f1r3node"
                             for call in capture.call_args_list))

    def test_missing_checkout_is_recorded_instead_of_aborting_provenance(self):
        with patch.object(benchmark.subprocess, "run", side_effect=FileNotFoundError("missing checkout")):
            result = benchmark.capture(["git", "rev-parse", "HEAD"], benchmark.HOST_DEPENDENCY)
        self.assertEqual(result["exit_code"], 127)
        self.assertIn("missing checkout", result["output"])


class SmokeOptions(unittest.TestCase):
    def test_explicit_and_legacy_modes_keep_filters_and_one_test_flag(self):
        for arguments, requested in [
            (["--", "validate_strict_signed/16$"], True),
            (["--", "--test", "validate_strict_signed/16$"], False),
        ]:
            extra, smoke = benchmark.criterion_options(arguments, requested)
            self.assertTrue(smoke)
            self.assertEqual(extra.count("--test"), 1)
            self.assertIn("validate_strict_signed/16$", extra)

    def test_measurement_keeps_baseline_arguments(self):
        extra, smoke = benchmark.criterion_options(["--", "--baseline", "saved"])
        self.assertFalse(smoke)
        self.assertEqual(extra, ["--baseline", "saved"])

    def test_target_discovery_excludes_library_and_binary_harnesses(self):
        workspace = {
            "workspace_members": ["core"],
            "packages": [{
                "name": "cordial-miners-core", "id": "core",
                "targets": [
                    {"name": "cordial_miners_core", "kind": ["lib"]},
                    {"name": "node", "kind": ["bin"]},
                    {"name": "block_validation", "kind": ["bench"]},
                    {"name": "deploy_pool", "kind": ["bench"]},
                ],
            }],
        }
        with patch.object(benchmark, "capture", return_value={
            "exit_code": 0, "output": json.dumps(workspace),
        }):
            self.assertEqual(benchmark.benchmark_targets(["cordial-miners-core"]), [
                ("cordial-miners-core", "block_validation"),
                ("cordial-miners-core", "deploy_pool"),
            ])
            with self.assertRaisesRegex(ValueError, "Unknown benchmark"):
                benchmark.benchmark_targets(["cordial-miners-core"], "missing")


class ProgressAndInterruption(unittest.TestCase):
    def test_active_record_and_logs_visible_before_process_finishes(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            manifest = output / "metadata.json"
            metadata = {"runs": []}
            process = Mock()
            process.wait.return_value = 0

            def lines():
                self.assertEqual(json.loads(manifest.read_text())["runs"][0]["status"], "running")
                yield "fixture started\n"
                log = output / "cordial-miners-core-block_validation.log"
                self.assertEqual(log.read_text(), "fixture started\n")

            process.stdout = MagicMock()
            process.stdout.__iter__.side_effect = lines
            with patch.object(benchmark.subprocess, "Popen", return_value=process), contextlib.redirect_stdout(io.StringIO()):
                code = benchmark.run_target(
                    "cordial-miners-core", "block_validation", ["--test"],
                    output, manifest, metadata,
                )
            self.assertEqual(code, 0)
            self.assertEqual(json.loads(manifest.read_text())["runs"][0]["status"], "passed")

    def test_interrupt_stops_children_and_records_partial_run(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            manifest = output / "metadata.json"
            metadata = {"runs": []}
            process = Mock(returncode=-signal.SIGTERM)
            process.stdout = MagicMock()
            process.stdout.__iter__.side_effect = KeyboardInterrupt
            with patch.object(benchmark.subprocess, "Popen", return_value=process), \
                    patch.object(benchmark, "stop_process_tree") as stop, \
                    contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(KeyboardInterrupt):
                    benchmark.run_target("cordial-miners-core", "block_validation",
                                         ["--test"], output, manifest, metadata)
            stop.assert_called_once_with(process)
            record = json.loads(manifest.read_text())["runs"][0]
            self.assertEqual(record["status"], "interrupted")
            self.assertEqual(record["exit_code"], -signal.SIGTERM)
            self.assertIn("finished_at_utc", record)

    @unittest.skipUnless(os.name == "posix", "Ubuntu/WSL process groups")
    def test_stubborn_process_group_is_killed_after_grace_period(self):
        process = Mock(pid=12345)
        process.wait.side_effect = [subprocess.TimeoutExpired("cargo", 5), 0]
        with patch.object(benchmark.os, "killpg") as kill:
            benchmark.stop_process_tree(process)
        self.assertEqual(kill.call_args_list, [
            call(12345, signal.SIGTERM), call(12345, signal.SIGKILL),
        ])

    def test_smoke_does_not_copy_stale_criterion_reports(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "smoke"
            with patch.object(sys, "argv", ["benchmark.py", "--smoke", "--output", str(output)]), \
                    patch.dict(os.environ, {}, clear=True), \
                    patch.object(benchmark, "provenance", return_value={}), \
                    patch.object(benchmark, "benchmark_targets", return_value=[("cordial-miners-core", "deploy_pool")]), \
                    patch.object(benchmark, "run_target", return_value=0), \
                    patch.object(benchmark.shutil, "copytree") as copy, \
                    contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(benchmark.main(), 0)
            copy.assert_not_called()
            metadata = json.loads((output / "metadata.json").read_text())
            self.assertTrue(metadata["configuration"]["smoke"])
            self.assertEqual(metadata["status"], "passed")
            self.assertFalse((output / "criterion").exists())


if __name__ == "__main__":
    unittest.main()
