#!/usr/bin/env python3
"""Validate the tested source and release plan before publishing desktop assets."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess

VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?")
SOURCE = re.compile(r"[0-9a-f]{40}")


def validate_run(event, repository, checkout):
    run = event.get("workflow_run", {})
    if (run.get("conclusion") != "success" or run.get("event") != "push"
            or run.get("head_branch") != "main"
            or run.get("name") != "Desktop quality and release artifacts"
            or run.get("head_repository", {}).get("full_name") != repository):
        raise ValueError("Only a successful main push from this repository can release")
    source = run.get("head_sha", "")
    if not SOURCE.fullmatch(source) or source != checkout:
        raise ValueError("The current main revision has not passed this quality run")
    return source


def validate_plan(plan, source):
    if (not isinstance(plan, dict) or plan.get("schema") != 1 or "version" not in plan
            or plan.get("source") != source or not SOURCE.fullmatch(source)
            or plan.get("scope") != "desktop"
            or plan.get("targets") != ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"]):
        raise ValueError("Release plan does not match the tested desktop source")
    version = plan.get("version")
    if version is not None and (not isinstance(version, str) or not VERSION.fullmatch(version)):
        raise ValueError("Release plan contains an invalid version")
    return version


def validate_version(version, expected):
    if not VERSION.fullmatch(version) or version != expected:
        raise ValueError("The release version changed after the artifacts were tested")


def output(name, value):
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as stream:
        stream.write(f"{name}={value}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("source")
    plan = subparsers.add_parser("plan")
    plan.add_argument("path", type=Path)
    version = subparsers.add_parser("version")
    version.add_argument("value")
    args = parser.parse_args()
    if args.command == "source":
        event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))
        checkout = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
        output("source", validate_run(event, os.environ["GITHUB_REPOSITORY"], checkout))
    elif args.command == "plan":
        plan = json.loads(args.path.read_text(encoding="utf-8"))
        output("version", validate_plan(plan, os.environ["SHEP_RELEASE_SOURCE"]) or "")
    else:
        validate_version(args.value, os.environ.get("SHEP_EXPECTED_VERSION", ""))


if __name__ == "__main__":
    main()
