"""Ensure the actual Lean executable rejects schema and observable mutations."""
import copy
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "lean/.lake/build/bin/slashing_replay"
FIXTURES = ROOT / "lean/slashing/fixtures.v1.json"


class SlashingReplayMutations(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.suite = json.loads(FIXTURES.read_text())

    def rejects(self, data, diagnostic):
        with tempfile.TemporaryDirectory(prefix="slashing-replay-") as directory:
            path = Path(directory) / "fixtures.json"
            path.write_text(json.dumps(data))
            result = subprocess.run(
                [str(RUNNER), str(path)], capture_output=True, text=True, timeout=30
            )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(diagnostic, result.stderr)

    def fixture(self, suite, name="mixed_penalties"):
        return next(f for f in suite["fixtures"] if f["name"] == name)

    def test_observable_mutations(self):
        for field in ["round", "active", "retained", "excluded", "weight", "penalty", "status"]:
            with self.subTest(field=field):
                suite = copy.deepcopy(self.suite)
                expected = self.fixture(suite)["expected"]
                if field == "round":
                    expected["state"]["round"] += 1
                elif field in ["active", "retained"]:
                    expected["state"]["entries"][0][field] += 1
                elif field == "excluded":
                    expected["state"]["entries"][0][field] = False
                elif field == "weight":
                    expected["weights"][0][field] += 1
                elif field == "penalty":
                    expected["tiers"][0][field] += 1
                else:
                    expected["status"] = "invalid_penalty_events"
                self.rejects(suite, {
                    "weight": "exported weight mismatch", "penalty": "penalty tier mismatch",
                    "status": "status", "round": "successor/atomicity mismatch",
                }.get(field, "successor/atomicity mismatch"))

    def test_rejection_and_atomicity_mutations(self):
        for field in ["status", "round", "retained"]:
            with self.subTest(field=field):
                suite = copy.deepcopy(self.suite)
                expected = self.fixture(suite, "already_ejected")["expected"]
                if field == "status":
                    expected[field] = "ok"
                elif field == "round":
                    expected["state"][field] += 1
                else:
                    expected["state"]["entries"][0][field] += 1
                self.rejects(suite, "status" if field == "status" else "successor/atomicity mismatch")

    def test_schema_mutations(self):
        for change in ["version", "boundary", "missing", "unknown", "nested_unknown",
                       "string_integer", "float_integer", "negative_integer", "u64_overflow",
                       "wrong_bool", "unsorted", "duplicate", "empty_suite"]:
            with self.subTest(change=change):
                suite = copy.deepcopy(self.suite)
                fixture = self.fixture(suite)
                if change == "version":
                    suite["schema_version"] = 2
                elif change == "boundary":
                    suite["boundary"] = "future"
                elif change == "missing":
                    del fixture["expected"]["tiers"]
                elif change == "unknown":
                    suite["extra"] = 0
                elif change == "nested_unknown":
                    fixture["events"]["inactivity"][0]["extra"] = 0
                elif change in ["string_integer", "float_integer", "negative_integer", "u64_overflow"]:
                    fixture["config"]["scale"] = {
                        "string_integer": "1000", "float_integer": 1000.5,
                        "negative_integer": -1, "u64_overflow": 2**64,
                    }[change]
                elif change == "wrong_bool":
                    fixture["previous"]["entries"][0]["excluded"] = 0
                elif change == "unsorted":
                    fixture["previous"]["entries"].reverse()
                elif change == "duplicate":
                    fixture["previous"]["entries"].insert(0, fixture["previous"]["entries"][0])
                else:
                    suite["fixtures"] = []
                self.rejects(suite, "slashing conformance FAILED")

    def test_candidate_and_event_mutations(self):
        suite = copy.deepcopy(self.suite)
        self.fixture(suite)["events"]["inactivity"][0]["missed_rounds"] = 2
        self.rejects(suite, "status")
        suite = copy.deepcopy(self.suite)
        self.fixture(suite)["candidate"]["entries"][-1]["active"] += 1
        self.rejects(suite, "successor/atomicity mismatch")


if __name__ == "__main__":
    unittest.main()
