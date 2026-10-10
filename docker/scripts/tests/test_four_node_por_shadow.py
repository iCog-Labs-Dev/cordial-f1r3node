"""Checks for the four-runner PoR smoke harness without a live cluster."""

import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "verify-four-node-por-shadow.py"
SPEC = importlib.util.spec_from_file_location("por_shadow_verify", SCRIPT)
VERIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFY)


def status(hashes=("genesis", "a", "b"), commitment="commitment", round_number=0):
    return {
        "active_por_round": round_number,
        "weight_commitment": commitment,
        "weights": {"validator": 200_000_000},
        "finalized_hashes": list(hashes),
        "finalized_anchor": hashes[-1] if hashes else None,
        "source_height": 3,
    }


class FourNodeShadowChecks(unittest.TestCase):
    def test_accepts_common_prefix_with_one_lagging_node(self):
        statuses = {node: status() for node in VERIFY.NODES}
        statuses[VERIFY.NODES[3]] = status(("genesis", "a"))
        self.assertEqual(VERIFY.compare_statuses(statuses, 2), 2)

    def test_rejects_different_finalized_prefix(self):
        statuses = {node: status() for node in VERIFY.NODES}
        statuses[VERIFY.NODES[1]] = status(("genesis", "other", "b"))
        with self.assertRaisesRegex(ValueError, "different finalized prefix"):
            VERIFY.compare_statuses(statuses, 2)

    def test_rejects_different_weight_commitment(self):
        statuses = {node: status() for node in VERIFY.NODES}
        statuses[VERIFY.NODES[2]] = status(commitment="other")
        with self.assertRaisesRegex(ValueError, "different PoR round, commitment, or weights"):
            VERIFY.compare_statuses(statuses, 2)

    def test_restart_preserves_prefix_and_projection(self):
        VERIFY.check_recovery(status(), status(("genesis", "a", "b", "c")))
        with self.assertRaisesRegex(ValueError, "finalized prefix changed"):
            VERIFY.check_recovery(status(), status(("genesis", "other", "b")))
        with self.assertRaisesRegex(ValueError, "projection changed"):
            VERIFY.check_recovery(status(), status(commitment="other"))

    def test_harness_restarts_runner_and_node_with_persistent_directories(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fake_runner = root / "fake-runner"
            fake_runner.write_text("""#!/usr/bin/env python3
import json
from pathlib import Path
import sys
import time

directory = Path(sys.argv[sys.argv.index('--data-dir') + 1]) / 'por'
directory.mkdir(parents=True, exist_ok=True)
status = {
    'schema_version': 1, 'shadow_mode': True, 'active_por_round': 0,
    'weight_commitment': 'commitment', 'weights': {'validator': 200000000},
    'finalized_hashes': ['genesis', 'a'], 'finalized_anchor': 'a',
    'source_height': 2,
}
while True:
    (directory / 'shadow-status.json').write_text(json.dumps(status))
    time.sleep(0.05)
""")
            fake_runner.chmod(0o755)
            fake_docker = root / "docker"
            fake_docker.write_text("#!/bin/sh\nexit 0\n")
            fake_docker.chmod(0o755)
            result = subprocess.run(
                ["python3", str(SCRIPT), "--data-root", str(root / "data"),
                 "--binary", str(fake_runner), "--timeout-seconds", "8"],
                env={**os.environ, "PATH": f"{root}:{os.environ['PATH']}"},
                capture_output=True, text=True, timeout=15,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.count("PASS:"), 3)


if __name__ == "__main__":
    unittest.main()
