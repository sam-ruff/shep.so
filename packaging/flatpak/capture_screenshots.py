#!/usr/bin/env python3
"""Capture the AppStream screenshots from fictional fixture data through the native MCP harness.

Needs the test-ui build and the harness tools described in the shep-e2e skill.
The tour is the saved `test_store_screenshots_*` scenario in scripts/e2e.py.
"""
import argparse
import importlib.util
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
OUTPUT = Path(__file__).with_name("screenshots")


def load_e2e():
    spec = importlib.util.spec_from_file_location("shep_e2e", ROOT / "scripts/e2e.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def convert(source, target, run=subprocess.run):
    tool = shutil.which("magick") or shutil.which("convert")
    if not tool:
        raise RuntimeError("ImageMagick is needed to convert the harness WebP captures to PNG")
    run([tool, str(source), "-strip", str(target)], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--output", type=Path, default=OUTPUT, help="Directory for the PNG screenshots")
    args = parser.parse_args()
    e2e = load_e2e()
    client = e2e.McpClient()
    try:
        artifacts = Path(client.call("desktop.start")["artifacts"])
        client.batch(*e2e.store_screenshot_tour())
        args.output.mkdir(parents=True, exist_ok=True)
        for name in e2e.STORE_SCREENSHOTS:
            convert(artifacts / f"{name}.webp", args.output / f"{name}.png")
            print(args.output / f"{name}.png")
    except (OSError, RuntimeError, AssertionError, subprocess.SubprocessError) as error:
        sys.exit(f"Screenshot capture failed: {error}")
    finally:
        client.close()


if __name__ == "__main__":
    main()
