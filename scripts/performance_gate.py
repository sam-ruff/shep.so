#!/usr/bin/env python3
"""Fail CI on missing, invalid, undersampled, or over-budget timing evidence."""
import json
import math
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


def main():
    budgets = json.loads((ROOT / "performance-budgets.json").read_text())
    directory = ROOT / "artifacts" / "performance"
    try:
        backend = json.loads((directory / "backend.json").read_text())
        ui = json.loads((directory / "ui.json").read_text())
    except (FileNotFoundError, json.JSONDecodeError) as error:
        sys.exit(f"Missing performance evidence. Run cargo bench --bench responsiveness and python3 scripts/e2e.py. {error}")
    errors = evaluate(budgets, backend, ui)
    if errors:
        sys.exit("Performance gate FAILED:\n" + "\n".join(errors))
    print("All performance gates passed.")


if __name__ == "__main__":
    main()
