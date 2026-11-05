#!/usr/bin/env python3
"""Focused checks for the keyed four-observer convergence gate."""

import copy
import json
from pathlib import Path
import runpy
import tempfile
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

    def test_recovery_requires_new_finalized_blocks_for_progress(self):
        before = self.statuses()[NODES[0]]
        after = copy.deepcopy(before)
        SCRIPT["check_recovery"](before, after)
        with self.assertRaisesRegex(ValueError, "post-restart finality"):
            SCRIPT["check_progress"](before, after)
        after["finalized_hashes"].append("new-finalized")
        after["finalized_anchor"] = "new-finalized"
        SCRIPT["check_progress"](before, after)
        with self.assertRaisesRegex(ValueError, "post-restart finality"):
            SCRIPT["check_progress"](before, after, 2)

    def test_requires_non_genesis_round_and_same_checkpoint(self):
        statuses = self.statuses()
        self.assertEqual(COMPARE(statuses, 2, 1), 2)

        statuses[NODES[0]]["active_por_round"] = 0
        with self.assertRaisesRegex(ValueError, "waiting for PoR round 1"):
            COMPARE(statuses, 2, 1)

        statuses = self.statuses()
        statuses[NODES[1]]["active_por_checkpoint"] = "checkpoint-b"
        with self.assertRaisesRegex(ValueError, "different PoR round, checkpoint, commitment, weights, or penalty state"):
            COMPARE(statuses, 2, 1)

    def test_requires_active_unequal_weights_and_post_cutover_finality(self):
        statuses = self.statuses()
        for status in statuses.values():
            status["active_effective_wave"] = 13
            status["observed_finalized_wave"] = 13
            status["weights"]["validator-b"] = 200
        self.assertEqual(COMPARE(statuses, 2, 1, 0, 2, None, 1), 2)
        with self.assertRaisesRegex(ValueError, "distinct active PoR weights"):
            equal = copy.deepcopy(statuses)
            for status in equal.values():
                status["weights"]["validator-b"] = 100
            COMPARE(equal, 2, 1, 0, 2, None, 1)
        with self.assertRaisesRegex(ValueError, "finalize PoR wave 14"):
            COMPARE(statuses, 2, 1, 0, 2, None, 2)

    def test_requires_same_ejection_and_retained_reputation(self):
        statuses = self.statuses()
        for index, node in enumerate(NODES):
            statuses[node]["configured_rating_rater"] = f"validator-{index + 1}"
            statuses[node]["weights"] = {"validator-1": 100, "validator-2": 0}
            statuses[node]["ejected_keys"] = ["validator-2"]
            statuses[node]["retained_reputation"] = {"validator-2": 75}
        self.assertEqual(COMPARE(statuses, 2, 1, 0, 2, 2), 2)
        changed = copy.deepcopy(statuses)
        changed[NODES[3]]["retained_reputation"]["validator-2"] = 0
        with self.assertRaisesRegex(ValueError, "penalty state"):
            COMPARE(changed, 2, 1, 0, 2, 2)
        changed = copy.deepcopy(statuses)
        for status in changed.values():
            status["ejected_keys"] = []
        with self.assertRaisesRegex(ValueError, "waiting for validator 2 PoR ejection"):
            COMPARE(changed, 2, 1, 0, 2, 2)
        before = statuses[NODES[0]]
        after = copy.deepcopy(before)
        after["ejected_keys"] = []
        with self.assertRaisesRegex(ValueError, "ejection disappeared"):
            SCRIPT["check_recovery"](before, after)

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

    def test_checkpoint_audit_requires_same_signed_ratings_and_matches_candidate(self):
        statuses = self.statuses()
        window = [f"block-{index}" for index in range(16)]
        for status in statuses.values():
            status["active_por_round"] = 0
            status["finalized_hashes"] = window
            status["candidate"] = {
                "por_round": 1,
                "source": {"prefix_hash": "source-a"},
                "checkpoint_hash": "candidate-a",
                "weight_commitment": "candidate-weights-a",
                "weights": {"validator-a": 80},
            }
        audit = {
            "schema_version": 1, "por_round": 1,
            "source_prefix_len": 16, "source_prefix_hash": "source-a",
            "window_block_hashes": window,
            "ratings": [{"rater": "validator-a", "recipient": "validator-b",
                         "score": 100, "evidence_ref": "block-1", "signature": "signed"}],
            "checkpoint_hash": "candidate-a", "weight_commitment": "candidate-weights-a",
            "weights": {"validator-a": 80},
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for node in NODES:
                path = SCRIPT["audit_path"](root, node, 1)
                path.parent.mkdir(parents=True)
                path.write_text(json.dumps(audit))
            compare = SCRIPT["compare_checkpoint_audits"]
            self.assertEqual(compare(root, statuses, 1), audit)
            divergent = copy.deepcopy(audit)
            divergent["ratings"][0]["signature"] = "other-signed"
            SCRIPT["audit_path"](root, NODES[1], 1).write_text(json.dumps(divergent))
            with self.assertRaisesRegex(ValueError, "different checkpoint rating audit"):
                compare(root, statuses, 1)
            SCRIPT["audit_path"](root, NODES[1], 1).write_text(json.dumps(audit))
            statuses[NODES[2]]["candidate"]["weight_commitment"] = "other-weights"
            with self.assertRaisesRegex(ValueError, "differs from its PoR candidate"):
                compare(root, statuses, 1)

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
