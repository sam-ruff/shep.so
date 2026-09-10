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

    def test_html_gate_uses_independent_pixels_and_recomputes_percentiles(self):
        budget = {"minimum_samples": 20, "html_budgets_ms": {"warm_return": 50}}
        rows = [{"case": "warm_return", "cycle": i, "input_to_pixels_ms": 20,
                 "match": 1., "before_match": 0.} for i in range(20)]
        def check(rows):
            with contextlib.redirect_stdout(io.StringIO()):
                # A fabricated summary cannot conceal slower raw observations.
                return gate.evaluate_html(budget, {"readings": rows, "summary": {"p95_ms": 0}})
        self.assertEqual(check(rows), [])
        self.assertTrue(check(rows[:19]))
        self.assertTrue(check(rows + [rows[0]]))
        for field, value in [("input_to_pixels_ms", True), ("input_to_pixels_ms", -1),
                             ("input_to_pixels_ms", float("nan")), ("input_to_pixels_ms", float("inf")),
                             ("match", .5), ("before_match", 1), ("cycle", False)]:
            self.assertTrue(check([{**row, field: value} for row in rows]))
        self.assertTrue(check([{**row, "input_to_pixels_ms": 51} for row in rows]))
        self.assertTrue(check([]))
