"""Offline harness behavior tests. Never contact real nodes or Docker."""
import argparse
import json
from pathlib import Path
import sys
import tempfile
import unittest
import urllib.error
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import multi_node_load as load

class LoadOutcomes(unittest.IsolatedAsyncioTestCase):
    async def exercise(self, behavior):
        nodes = ["http://node-a", "http://node-b"]
        args = argparse.Namespace(
            concurrency=4, node=nodes, container=["node-a", "node-b"],
            request_timeout=.05, rate=100, settle_seconds=.3, poll_interval=.001,
        )
        submitted = False
        def http(url, timeout, payload=None):
            nonlocal submitted
            if url.endswith("/api/status"):
                return {"isReady": True}
            if payload is not None:
                submitted = True
                return {"success": behavior != "rejected"}
            if not submitted:
                raise urllib.error.HTTPError(url, 404, "not found", {}, None)
            if behavior == "missing_node" and "node-b" in url:
                return {"deployId": "aa", "isFinalized": False, "errored": False}
            if behavior == "unavailable_node" and "node-b" in url:
                raise OSError("node unavailable")
            return {
                "deployId": "aa", "isFinalized": True, "errored": behavior == "execution_error",
                "blockHash": "other" if behavior == "conflicting_hashes" and "node-b" in url else "block",
            }
        resources = [
            {"container": n, "cpu_percent": 5, "memory_usage_bytes": 1024}
            for n in args.container
        ]
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            report = {}
            with patch.object(load, "request_json", side_effect=http), patch.object(load, "docker_stats", return_value=resources):
                code = await load.run(args, [{"signature": "aa"}], output, report)
            rows = [json.loads(line) for line in (output / "deploys.jsonl").read_text().splitlines()]
            self.assertEqual(len(rows), 1)
            return code, report, rows[0]

    async def test_requires_finality_on_every_node(self):
        code, report, row = await self.exercise("missing_node")
        self.assertEqual(code, 2)
        self.assertFalse(report["complete"])
        self.assertEqual(report["finalized_on_all_nodes"], 0)
        self.assertEqual(report["per_node"]["http://node-a"]["finalized"], 1)
        self.assertEqual(row["outcome"], "finalization_timeout")
        self.assertIsNone(report["finalization_latency_seconds"]["p95"])

    async def test_rejection_is_not_counted_as_acceptance(self):
        code, report, row = await self.exercise("rejected")
        self.assertEqual(code, 2)
        self.assertEqual(report["accepted"], 0)
        self.assertEqual(report["finalized_on_all_nodes"], 0)

    async def test_execution_failure_is_reported_separately(self):
        code, report, row = await self.exercise("execution_error")
        self.assertEqual(code, 2)
        self.assertEqual(report["finalized_on_all_nodes"], 1)
        self.assertEqual(report["successful_execution_finalized"], 0)
        self.assertEqual(report["finalized_execution_errors"], 1)

    async def test_conflicting_finalized_blocks_fail(self):
        code, report, row = await self.exercise("conflicting_hashes")
        self.assertEqual(code, 2)
        self.assertEqual(row["outcome"], "inconsistent_finalized_blocks")
        self.assertEqual(report["finalized_on_all_nodes"], 0)

    async def test_observation_failure_never_fabricates_latency(self):
        code, report, row = await self.exercise("unavailable_node")
        self.assertEqual(code, 2)
        self.assertGreater(row["poll_errors"], 0)
        self.assertEqual(report["finalization_latency_seconds"]["count"], 0)

    async def test_success_is_finalized_not_merely_accepted(self):
        code, report, row = await self.exercise("success")
        self.assertEqual(code, 0)
        self.assertTrue(report["complete"])
        self.assertEqual(report["successful_execution_finalized"], 1)
        self.assertEqual(set(row["finalized"]), {"http://node-a", "http://node-b"})

if __name__ == "__main__":
    unittest.main()
