#!/usr/bin/env python3
"""Measure real ten-message Delete controls in a 100,000-message native mailbox."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import statistics
import time

from e2e import McpClient, ROOT, check, click, mail_row_y, shot, wait


def select_ten(mcp):
    mcp.batch(check("total", 100000),
              click(400, mail_row_y(0)), check("selected", "A little more room to think"),
              {"type": "click", "x": 400, "y": mail_row_y(0), "modifiers": ["ctrl"]},
              check("mail_selection.mode", True), check("mail_selection.drawn", True),
              {"type": "click", "x": 400, "y": mail_row_y(9), "modifiers": ["shift"]},
              check("mail_selection.count", 10), check("mail_selection.pending", False), wait(200))


def baseline(mcp):
    return mcp.batch({"type": "pixel_baseline"})["actions"][0]["result"]["baseline"]


def reference(mcp, token, regions):
    return mcp.batch({"type": "changed_pixel_reference", "baseline": token,
                      "regions": regions})["actions"][0]["result"]["points"]


def confirmed(mcp):
    mcp.batch(check("dialog", None), check("total", 99990),
              check("action_toast.label", "Deleted 10 messages"),
              check("bulk.jobs.0.total", 10), check("bulk.jobs.0.completed", 0))


def undo(mcp):
    mcp.batch(click(1327, 872), check("total", 100000), check("bulk.jobs.0.undo_requested", True),
              check("mail_selection.mode", False), wait(200))


def measure(samples, prepare_only=False):
    binary = ROOT / "target/test-ui/shep"
    fingerprint = hashlib.sha256(binary.read_bytes()).hexdigest()
    mcp = McpClient()
    readings = []
    try:
        started = mcp.call("desktop.start", selection_mailbox=True, held_provider_slots=True, store_truth=False)
        mcp.batch(check("store_truth.enabled", False))
        select_ten(mcp)
        token = baseline(mcp)
        mcp.batch(click(696, 100), check("dialog", "BulkReview"), check("bulk.review_count", 10),
                  wait(200), shot("review-reference"))
        review = reference(mcp, token, [[462, 448, 300, 25], [871, 527, 96, 20]])
        token = baseline(mcp)
        mcp.batch(click(927, 533))
        confirmed(mcp)
        mcp.batch(wait(200), shot("confirmation-reference"))
        confirmation = reference(mcp, token, [[270, 225, 140, 18], [320, 27, 100, 20],
                                               [1160, 863, 145, 18]])
        undo(mcp)
        if prepare_only:
            return {"preparation_only": True, "binary_sha256": fingerprint,
                    "artifacts": started["artifacts"], "review_points": review,
                    "confirmation_points": confirmation}
        for cycle in range(samples):
            select_ten(mcp)
            for case, x, y, points in (("review_ready", 696, 100, review),
                                       ("confirmation_feedback", 927, 533, confirmation)):
                result = mcp.batch({"type": "measure_pixels", "x": x, "y": y,
                                    "points": points, "require_change": True, "timeout_ms": 5000})
                state = result["state"]
                if len(state["mail_rows"]) > 50 or state["cache_entries"] > 8:
                    raise RuntimeError("The action exceeded the bounded metadata/body cache contract.")
                reading = {"case": case, "cycle": cycle, **result["actions"][0]["result"],
                           "ui_handler_p95_ms": state["update_p95_ms"],
                           "metadata_rows": len(state["mail_rows"]), "cached_bodies": state["cache_entries"]}
                readings.append(reading)
                print(json.dumps(reading), flush=True)
                if case == "review_ready":
                    mcp.batch(check("dialog", "BulkReview"), check("bulk.review_count", 10))
                else:
                    confirmed(mcp)
            mcp.batch(shot(f"confirmed-{cycle}"))
            undo(mcp)
        summary = {}
        for case in ("review_ready", "confirmation_feedback"):
            values = sorted(r["input_to_pixels_ms"] for r in readings if r["case"] == case)
            summary[case] = {"count": len(values), "p50_ms": statistics.median(values),
                             "p95_ms": values[math.ceil(len(values) * .95) - 1], "max_ms": max(values)}
        if hashlib.sha256(binary.read_bytes()).hexdigest() != fingerprint:
            raise RuntimeError("The measured binary changed; rebuild and repeat the quiet run.")
        return {"schema": 1, "timestamp": time.time(), "binary_sha256": fingerprint,
                "mailbox_messages": 100000, "selected_messages": 10,
                "provider_capacity": "held throughout", "store_truth_oracle": False,
                "artifacts": started["artifacts"],
                "method": "Real XTest button press/release to 97% of changed foreground reference pixels in each UI stage; RGB tolerance 8, 2ms polling. Setup and reference preparation excluded. Quiet host must be arranged separately.",
                "summary": summary, "readings": readings}
    finally:
        mcp.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--samples", type=int, default=20)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--prepare-only", action="store_true",
                        help="Check the fixture, real controls and changed-region references without timing.")
    args = parser.parse_args()
    if not 1 <= args.samples <= 25:
        parser.error("samples must be 1–25")
    report = measure(args.samples, args.prepare_only)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    if args.prepare_only:
        raise SystemExit(0)
    from performance_gate import evaluate_actions
    budgets = json.loads((ROOT / "performance-budgets.json").read_text())
    errors = evaluate_actions(budgets, report)
    if errors:
        raise SystemExit("Action performance gate FAILED:\n" + "\n".join(errors))
