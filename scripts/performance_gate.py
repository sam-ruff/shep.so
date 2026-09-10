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


def evaluate_html(budgets, report):
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
        if p95 > limit:
            errors.append(f"HTML {case}: {p95:.3f} ms exceeds {limit:.3f} ms")
        else:
            print(f"PASS HTML {case}: {p95:.3f} / {limit:.3f} ms ({len(values)} samples)")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--html-only", action="store_true", help="Check the separately authorized HTML measurements only")
    parser.add_argument("--html-report", type=Path, default=ROOT / "artifacts/performance/html.json")
    args = parser.parse_args()
    budgets = json.loads((ROOT / "performance-budgets.json").read_text())
    directory = ROOT / "artifacts" / "performance"
    try:
        html = json.loads(args.html_report.read_text())
        errors = evaluate_html(budgets, html)
        if not args.html_only:
            backend = json.loads((directory / "backend.json").read_text())
            ui = json.loads((directory / "ui.json").read_text())
            errors.extend(evaluate(budgets, backend, ui))
    except (FileNotFoundError, json.JSONDecodeError) as error:
        sys.exit(f"Missing performance evidence. Run cargo bench --bench responsiveness, python3 scripts/e2e.py and python3 scripts/html_latency.py --output artifacts/performance/html.json. {error}")
    if errors:
        sys.exit("Performance gate FAILED:\n" + "\n".join(errors))
    print("All performance gates passed.")


if __name__ == "__main__":
    main()
