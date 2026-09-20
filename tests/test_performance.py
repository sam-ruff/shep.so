import contextlib
import importlib.util
import io
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("gate", Path(__file__).resolve().parents[1] / "scripts/performance_gate.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class PerformanceGate(unittest.TestCase):
    def action_report(self):
        return {"schema": 1, "binary_sha256": "a" * 64, "mailbox_messages": 100000, "selected_messages": 10,
                "provider_capacity": "held throughout", "store_truth_oracle": False,
                "summary": {"p95_ms": 0},
                "readings": [{"case": case, "cycle": cycle, "input_to_pixels_ms": 20,
                              "match": 1., "before_match": 0., "ui_handler_p95_ms": .1,
                              "metadata_rows": 50, "cached_bodies": 6}
                             for case in ("review_ready", "confirmation_feedback")
                             for cycle in range(20)]}

    def evaluate_actions(self, report):
        budgets = {"dataset_messages": 100000, "minimum_samples": 20,
                   "budgets_ms": {"ui_handler_p95": 8},
                   "action_budgets_ms": {"review_ready": 100, "confirmation_feedback": 100}}
        with contextlib.redirect_stdout(io.StringIO()):
            return gate.evaluate_actions(budgets, report)

    def test_action_gate_accepts_paired_real_pixel_observations(self):
        self.assertEqual(self.evaluate_actions(self.action_report()), [])

    def test_action_gate_rejects_wrong_fixture_and_observer_conditions(self):
        for field, value in (("schema", True), ("schema", 2), ("binary_sha256", None),
                             ("binary_sha256", "g" * 64), ("binary_sha256", "a" * 63), ("mailbox_messages", 99999),
                             ("mailbox_messages", True), ("selected_messages", 9),
                             ("selected_messages", 20), ("provider_capacity", "available"),
                             ("store_truth_oracle", True), ("store_truth_oracle", None),
                             ("readings", None)):
            with self.subTest(field=field, value=value):
                report = self.action_report()
                report[field] = value
                self.assertTrue(self.evaluate_actions(report))

    def test_action_gate_rejects_invalid_pixels_and_unbounded_retention(self):
        for field, value in (("input_to_pixels_ms", True), ("input_to_pixels_ms", -1),
                             ("input_to_pixels_ms", float("nan")), ("input_to_pixels_ms", float("inf")),
                             ("input_to_pixels_ms", 101), ("match", .5), ("before_match", .11),
                             ("cycle", False), ("ui_handler_p95_ms", 8.1),
                             ("ui_handler_p95_ms", None), ("metadata_rows", 51),
                             ("metadata_rows", True), ("cached_bodies", 9), ("cached_bodies", -1)):
            with self.subTest(field=field, value=value):
                report = self.action_report()
                report["readings"] = [{**row, field: value} for row in report["readings"]]
                self.assertTrue(self.evaluate_actions(report))

    def test_action_gate_requires_twenty_unique_paired_cycles(self):
        report = self.action_report()
        report["readings"].pop()
        self.assertTrue(self.evaluate_actions(report))
        report = self.action_report()
        report["readings"].append(report["readings"][0])
        self.assertTrue(self.evaluate_actions(report))
        report = self.action_report()
        report["readings"][-1]["cycle"] = 20
        self.assertTrue(self.evaluate_actions(report))
        for row in (None, {"case": []}, {"case": "unknown"}):
            report = self.action_report()
            report["readings"].append(row)
            self.assertTrue(self.evaluate_actions(report))

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
