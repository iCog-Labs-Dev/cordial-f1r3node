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

    def test_complete_checkpoint_candidate_converges_before_activation(self):
        statuses = self.statuses()
        for status in statuses.values():
            status["active_por_round"] = 0
            status["active_por_checkpoint"] = None
            status["candidate"] = {
                "por_round": 1,
                "source": {"por_round": 1, "finalized_prefix_len": 16,
                           "prefix_hash": "source-a", "terminal_block_hash": "block-a"},
                "checkpoint_hash": "candidate-a",
                "weight_commitment": "candidate-weights-a",
                "weights": {"validator-a": 80},
            }
        self.assertEqual(COMPARE(statuses, 2, 0, 1), 2)

        before = copy.deepcopy(statuses[NODES[0]])
        SCRIPT["check_recovery"](before, copy.deepcopy(before))
        changed = copy.deepcopy(before)
        changed["candidate"]["source"]["prefix_hash"] = "source-b"
        statuses[NODES[1]] = changed
        with self.assertRaisesRegex(ValueError, "different PoR checkpoint candidate"):
            COMPARE(statuses, 2, 0, 1)
        with self.assertRaisesRegex(ValueError, "candidate changed"):
            SCRIPT["check_recovery"](before, changed)

        changed = copy.deepcopy(before)
        changed["candidate"] = None
        with self.assertRaisesRegex(ValueError, "waiting for complete PoR candidate"):
            COMPARE({node: copy.deepcopy(changed) for node in NODES}, 2, 0, 1)

    def test_candidate_can_become_the_active_projection_after_restart(self):
        before = self.statuses()[NODES[0]]
        before["active_por_round"] = 0
        before["active_por_checkpoint"] = None
        before["candidate"] = {
            "por_round": 1, "source": {"por_round": 1},
            "checkpoint_hash": "checkpoint-a",
            "weight_commitment": "weights-a",
            "weights": {"validator-a": 100},
        }
        after = copy.deepcopy(before)
        after["active_por_round"] = 1
        after["active_por_checkpoint"] = "checkpoint-a"
        after["candidate"] = None
        SCRIPT["check_recovery"](before, after)

        after["weight_commitment"] = "wrong-weights"
        with self.assertRaisesRegex(ValueError, "differs from the prior candidate"):
            SCRIPT["check_recovery"](before, after)


if __name__ == "__main__":
    unittest.main()
