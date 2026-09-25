#!/usr/bin/env python3
"""Fail CI on missing, invalid, undersampled, or over-budget timing evidence."""
import json
import math
import argparse
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]


def evaluate(budgets, backend, ui):
    errors = []
    if backend.get("dataset_messages", 0) < budgets["dataset_messages"]:
        errors.append("Backend benchmark used too few messages")
    for report in (backend, ui):
        if report.get("samples", 0) < budgets["minimum_samples"]:
            errors.append("Performance report has too few samples")
    metrics = {**backend.get("metrics_ms", {}), **ui.get("metrics_ms", {})}
    for name, limit in budgets["budgets_ms"].items():
        value = metrics.get(name)
        if not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
            errors.append(f"{name}: missing or invalid measurement")
        elif value > limit:
            errors.append(f"{name}: {value:.3f} ms exceeds {limit:.3f} ms")
        else:
            print(f"PASS {name}: {value:.3f} / {limit:.3f} ms")
    return errors


def evaluate_html(budgets, report, enforce=True):
    errors = []
    readings = report.get("readings", [])
    if not isinstance(readings, list):
        return ["HTML report has no valid readings"]
    valid_number = lambda value: type(value) in (int, float) and math.isfinite(value)
    for case, limit in budgets["html_budgets_ms"].items():
        rows = [r for r in readings if isinstance(r, dict) and r.get("case") == case]
        cycles, values = set(), []
        for row in rows:
            value, cycle = row.get("input_to_pixels_ms"), row.get("cycle")
            match, before = row.get("match"), row.get("before_match")
            if (not valid_number(value) or value < 0 or type(cycle) is not int or cycle < 0
                    or cycle in cycles or not valid_number(match) or not .97 <= match <= 1
                    or not valid_number(before) or not 0 <= before < .97):
                errors.append(f"HTML {case}: invalid or duplicate pixel observation")
                continue
            cycles.add(cycle)
            values.append(value)
        if len(values) < budgets["minimum_samples"]:
            errors.append(f"HTML {case}: too few valid pixel observations")
            continue
        p95 = sorted(values)[math.ceil(len(values)*.95)-1]
        if p95 > limit and enforce:
            errors.append(f"HTML {case}: {p95:.3f} ms exceeds {limit:.3f} ms")
        elif p95 > limit:
            print(f"OVER HTML {case}: {p95:.3f} / {limit:.3f} ms ({len(values)} samples, report only)")
        else:
            print(f"PASS HTML {case}: {p95:.3f} / {limit:.3f} ms ({len(values)} samples)")
    return errors


def evaluate_actions(budgets, report):
    if not isinstance(report, dict) or type(report.get("schema")) is not int or report["schema"] != 1:
        return ["Action report has no supported schema"]
    errors = []
    messages = report.get("mailbox_messages")
    fingerprint = report.get("binary_sha256")
    if (not isinstance(fingerprint, str) or len(fingerprint) != 64
            or any(character not in "0123456789abcdef" for character in fingerprint)):
        errors.append("Action report has no valid measured binary fingerprint")
    if type(messages) is not int or messages < budgets["dataset_messages"]:
        errors.append("Action benchmark used too few messages")
    if type(report.get("selected_messages")) is not int or report["selected_messages"] != 10:
        errors.append("Action benchmark must select exactly ten messages")
    if report.get("provider_capacity") != "held throughout" or report.get("store_truth_oracle") is not False:
        errors.append("Action timing requires held provider capacity and a disabled full database oracle")
    readings = report.get("readings")
    if not isinstance(readings, list):
        return errors + ["Action report has no valid readings"]
    valid_number = lambda value: type(value) in (int, float) and math.isfinite(value)
    cases = budgets["action_budgets_ms"]
    if any(not isinstance(row, dict) or not isinstance(row.get("case"), str)
           or row["case"] not in cases for row in readings):
        errors.append("Action report contains an invalid observation")
    observed_cycles = []
    for case, limit in cases.items():
        cycles, values = set(), []
        for row in (row for row in readings if isinstance(row, dict) and row.get("case") == case):
            value, cycle = row.get("input_to_pixels_ms"), row.get("cycle")
            match, before = row.get("match"), row.get("before_match")
            handler = row.get("ui_handler_p95_ms")
            rows, bodies = row.get("metadata_rows"), row.get("cached_bodies")
            if (not valid_number(value) or value < 0 or type(cycle) is not int or cycle < 0
                    or cycle in cycles or not valid_number(match) or not .97 <= match <= 1
                    or not valid_number(before) or not 0 <= before <= .1
                    or not valid_number(handler) or not 0 <= handler <= budgets["budgets_ms"]["ui_handler_p95"]
                    or type(rows) is not int or not 0 <= rows <= 50
                    or type(bodies) is not int or not 0 <= bodies <= 8):
                errors.append(f"Action {case}: invalid, duplicate or unbounded pixel observation")
                continue
            cycles.add(cycle)
            values.append(value)
        observed_cycles.append(cycles)
        if len(values) < budgets["minimum_samples"]:
            errors.append(f"Action {case}: too few valid pixel observations")
            continue
        p95 = sorted(values)[math.ceil(len(values) * .95) - 1]
        if p95 > limit:
            errors.append(f"Action {case}: {p95:.3f} ms exceeds {limit:.3f} ms")
        else:
            print(f"PASS Action {case}: {p95:.3f} / {limit:.3f} ms ({len(values)} samples)")
    if any(cycles != observed_cycles[0] for cycles in observed_cycles[1:]):
        errors.append("Action review and confirmation cycles do not match")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    only = parser.add_mutually_exclusive_group()
    only.add_argument("--html-only", action="store_true", help="Check the separately authorized HTML measurements only")
    only.add_argument("--actions-only", action="store_true", help="Check the immediate-action pixel measurements only")
    parser.add_argument("--html-report-only", action="store_true",
                        help="Validate the HTML evidence but report over-budget timings without failing")
    parser.add_argument("--html-report", type=Path, default=ROOT / "artifacts/performance/html.json")
    parser.add_argument("--actions-report", type=Path, default=ROOT / "artifacts/performance/actions.json")
    args = parser.parse_args()
    budgets = json.loads((ROOT / "performance-budgets.json").read_text())
    directory = ROOT / "artifacts" / "performance"
    try:
        errors = []
        if not args.actions_only:
            html = json.loads(args.html_report.read_text())
            errors.extend(evaluate_html(budgets, html, enforce=not args.html_report_only))
        if not args.html_only:
            actions = json.loads(args.actions_report.read_text())
            errors.extend(evaluate_actions(budgets, actions))
        if not args.html_only and not args.actions_only:
            backend = json.loads((directory / "backend.json").read_text())
            ui = json.loads((directory / "ui.json").read_text())
            errors.extend(evaluate(budgets, backend, ui))
    except (FileNotFoundError, json.JSONDecodeError) as error:
        sys.exit(f"Missing performance evidence. Run cargo bench --bench responsiveness, python3 scripts/e2e.py, python3 scripts/html_latency.py --output artifacts/performance/html.json and python3 scripts/action_latency.py --output artifacts/performance/actions.json. {error}")
    if errors:
        sys.exit("Performance gate FAILED:\n" + "\n".join(errors))
    print("All performance gates passed.")


if __name__ == "__main__":
    main()
