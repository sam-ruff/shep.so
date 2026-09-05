import contextlib
import importlib.util
import io
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("gate", Path(__file__).resolve().parents[1] / "scripts/performance_gate.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class PerformanceGate(unittest.TestCase):
    def evaluate(self, value=3, samples=20, dataset=100000):
        with contextlib.redirect_stdout(io.StringIO()):
            return gate.evaluate({"dataset_messages": 100000, "minimum_samples": 20, "budgets_ms": {"test": 8}},
                                 {"dataset_messages": dataset, "samples": samples, "metrics_ms": {"test": value}},
                                 {"samples": 20})

    def test_valid_measurements_pass(self):
        self.assertEqual(self.evaluate(), [])

    def test_missing_invalid_undersampled_and_slow_evidence_fails(self):
        for value in (None, float("nan"), float("inf"), -1, 9):
            self.assertTrue(self.evaluate(value=value))
        self.assertTrue(self.evaluate(samples=19))
        self.assertTrue(self.evaluate(dataset=99999))
