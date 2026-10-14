#!/usr/bin/env python3
"""Focused checks for the keyed four-observer convergence gate."""

import copy
from pathlib import Path
import runpy
import unittest


SCRIPT = runpy.run_path(str(Path(__file__).with_name("verify-four-node-por-shadow.py")))
COMPARE = SCRIPT["compare_statuses"]
NODES = SCRIPT["NODES"]


class FourNodePorShadowGateTests(unittest.TestCase):
    def statuses(self):
        status = {
            "active_por_round": 1,
            "active_por_checkpoint": "checkpoint-a",
            "weight_commitment": "weights-a",
            "weights": {"validator-a": 100},
            "finalized_hashes": ["genesis", "finalized"],
            "finalized_anchor": "finalized",
            "source_height": 5,
        }
        return {node: copy.deepcopy(status) for node in NODES}

    def test_requires_non_genesis_round_and_same_checkpoint(self):
        statuses = self.statuses()
        self.assertEqual(COMPARE(statuses, 2, 1), 2)

        statuses[NODES[0]]["active_por_round"] = 0
        with self.assertRaisesRegex(ValueError, "waiting for PoR round 1"):
            COMPARE(statuses, 2, 1)

        statuses = self.statuses()
        statuses[NODES[1]]["active_por_checkpoint"] = "checkpoint-b"
        with self.assertRaisesRegex(ValueError, "different PoR round, checkpoint, commitment, or weights"):
            COMPARE(statuses, 2, 1)


if __name__ == "__main__":
    unittest.main()
