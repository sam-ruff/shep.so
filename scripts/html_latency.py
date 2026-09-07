#!/usr/bin/env python3
"""Repeatable MCP selection-to-presented-HTML measurements on fictional mail."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import statistics
import time

from e2e import McpClient, click, check, wait, shot, ROOT

MAIL = {
    "styled": (245, "Styled sign-in sample"),
    "xhtml": (351, "Mislabeled XHTML request"),
    "escaped": (454, "Escaped HTML request"),
    "long": (558, "Long formatted letter"),
}


def measure(samples=20):
    binary = ROOT / "target/test-ui/shep"
    fingerprint = hashlib.sha256(binary.read_bytes()).hexdigest()
    mcp = McpClient()
    references, evidence, readings = {}, [], []
    try:
        result = mcp.call("desktop.start", html_mail=True)
        evidence.append(result["artifacts"])
        for name, (y, subject) in MAIL.items():
            mcp.batch(click(400, y), check("selected", subject), check("html_ready", True), wait(500), shot(f"reference-{name}"))
            references[name] = mcp.batch({"type": "pixel_reference"})["actions"][0]["result"]
        for cycle in range(samples):
            # A fresh process/cache makes the nonadjacent long-letter first open
            # independent of earlier interactive or speculative rendering.
            result = mcp.call("desktop.start", html_mail=True)
            evidence.append(result["artifacts"])
            mcp.batch(check("html_ready", True), wait(300))
            for case, name in (("cold_nonadjacent", "long"), ("warm_return", "styled"),
                               ("prefetched_neighbor", "xhtml"), ("warm_repeat", "long")):
                y, subject = MAIL[name]
                result = mcp.batch({"type": "measure_pixels", "x": 400, "y": y,
                                    "points": references[name]["points"], "timeout_ms": 5000},
                                   check("selected", subject), check("html_ready", True))
                reading = {"case": case, "message": name, "cycle": cycle,
                           **result["actions"][0]["result"],
                           "html_cache_hits": result["state"].get("html_cache_hits"),
                           "artifacts": evidence[-1]}
                readings.append(reading)
                print(json.dumps(reading), flush=True)
                # A human dwell gives neighbor preparation an explicit chance;
                # it is outside the measured click-to-pixels interval.
                mcp.batch(wait(300))
            if cycle == 0:
                mcp.batch(shot("measured-long-letter"))
        summary = {}
        for case in sorted({r["case"] for r in readings}):
            values = sorted(r["input_to_pixels_ms"] for r in readings if r["case"] == case)
            summary[case] = {"count": len(values), "p50_ms": statistics.median(values),
                             "p95_ms": values[math.ceil(len(values)*.95)-1], "max_ms": max(values)}
        if hashlib.sha256(binary.read_bytes()).hexdigest() != fingerprint:
            raise RuntimeError("The test binary changed during measurement; rebuild first and rerun.")
        return {"schema": 1, "timestamp": time.time(), "binary_sha256": fingerprint,
                "method": "Native XTest click to >=97% of 64 exact X11 reference body pixels (RGB tolerance 8); 2ms poll sleep; 1440x920 RGB24 Xvfb; no artificial provider delay. Host not asserted idle.",
                "summary": summary, "readings": readings, "evidence": evidence}
    finally:
        mcp.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--samples", type=int, default=20)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not 1 <= args.samples <= 100:
        parser.error("samples must be 1–100")
    report = measure(args.samples)
    from html_image_latency import measure as measure_images
    illustrated = measure_images(args.samples)
    if report["binary_sha256"] != illustrated["binary_sha256"]:
        raise RuntimeError("The binary changed between HTML measurement groups.")
    report["readings"].extend(illustrated["readings"])
    report["evidence"].append(illustrated["artifacts"])
    for case in ("warm_images_return", "warm_images_repeat"):
        values = sorted(r["input_to_pixels_ms"] for r in illustrated["readings"] if r["case"] == case)
        report["summary"][case] = {"count": len(values), "p50_ms": statistics.median(values),
                                  "p95_ms": values[math.ceil(len(values)*.95)-1], "max_ms": max(values)}
    report["method"] += " Includes repeated reopening of two nested-table messages with twelve permitted fixture images each; both final pixels and renderer image acknowledgments are checked."
    from html_nested_latency import measure as measure_nested
    nested = measure_nested(args.samples)
    if report["binary_sha256"] != nested["binary_sha256"]:
        raise RuntimeError("The binary changed between HTML measurement groups.")
    report["readings"].extend(nested["readings"])
    report["evidence"].extend(nested["evidence"])
    for case in ("cold_nested_table", "warm_nested_table"):
        values = sorted(r["input_to_pixels_ms"] for r in nested["readings"] if r["case"] == case)
        report["summary"][case] = {"count": len(values), "p50_ms": statistics.median(values),
                                  "p95_ms": values[math.ceil(len(values)*.95)-1], "max_ms": max(values)}
    report["method"] += " Includes a fictional 16-level nested table with 1,182 utility CSS rules, cold and revisited."
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2)+"\n")
    print(json.dumps(report["summary"], indent=2))
