#!/usr/bin/env python3
"""Generate Flatpak offline Cargo sources from Cargo.lock.

The output matches flatpak-cargo-generator for crates.io packages: one
checksummed crate archive plus an inline `.cargo-checksum.json` per package,
and a Cargo configuration that replaces crates.io with the vendored directory.
Git and alternative registry sources are rejected, so every crate the build
needs is pinned by the checksum already recorded in Cargo.lock.
"""
import argparse
import json
from pathlib import Path
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
DOWNLOAD = "https://static.crates.io/crates"
CARGO_HOME = "cargo"
VENDOR = f"{CARGO_HOME}/vendor"
CARGO_CONFIG = (
    "[source.vendored-sources]\n"
    f'directory = "{VENDOR}"\n\n'
    "[source.crates-io]\n"
    'replace-with = "vendored-sources"\n'
)


def crate_sources(lock):
    """Return the flatpak-builder sources for every registry package in a parsed Cargo.lock."""
    sources = []
    for package in lock.get("package", []):
        source = package.get("source")
        if source is None:
            continue  # Workspace member or path dependency, built from the checkout.
        name, version = package["name"], package["version"]
        if source != CRATES_IO:
            raise ValueError(f"{name} {version} comes from {source}; only crates.io packages can be vendored")
        checksum = package.get("checksum")
        if not checksum:
            raise ValueError(f"{name} {version} has no checksum in Cargo.lock")
        dest = f"{VENDOR}/{name}-{version}"
        sources.append({
            "type": "archive",
            "archive-type": "tar-gzip",
            "url": f"{DOWNLOAD}/{name}/{name}-{version}.crate",
            "sha256": checksum,
            "dest": dest,
        })
        sources.append({
            "type": "inline",
            "contents": json.dumps({"package": checksum, "files": {}}),
            "dest": dest,
            "dest-filename": ".cargo-checksum.json",
        })
    sources.append({"type": "inline", "contents": CARGO_CONFIG, "dest": CARGO_HOME, "dest-filename": "config.toml"})
    return sources


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lock", type=Path, default=ROOT / "Cargo.lock", help="Cargo.lock to read")
    parser.add_argument("--output", type=Path, default=Path(__file__).with_name("cargo-sources.json"),
                        help="Sources file to write (default packaging/flatpak/cargo-sources.json)")
    args = parser.parse_args()
    try:
        with args.lock.open("rb") as lock:
            sources = crate_sources(tomllib.load(lock))
    except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
        sys.exit(f"Could not generate Cargo sources: {error}")
    args.output.write_text(json.dumps(sources, indent=4) + "\n", encoding="utf-8")
    print(f"Wrote {len(sources) // 2} crates to {args.output}")


if __name__ == "__main__":
    main()
