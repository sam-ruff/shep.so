#!/usr/bin/env python3
"""Type-check the exact native tray adapter without linking the complete app.

Requires `rustup target add aarch64-apple-darwin` (or the chosen Windows target).
This checks native API/type compatibility only; execute the app on each OS too.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=["aarch64-apple-darwin", "x86_64-pc-windows-gnu"], default="aarch64-apple-darwin")
    args = parser.parse_args()
    base = ROOT / "artifacts" / "tray-adapter-check"
    base.mkdir(parents=True, exist_ok=True)
    # A fixed isolated workspace retains its own default Cargo target for repeats.
    (base / "src").mkdir(exist_ok=True)
    (base / "Cargo.toml").write_text('''[package]
name = "shep-native-tray-adapter-check"
version = "0.0.0"
edition = "2024"
publish = false
[workspace]
[dependencies]
anyhow = "1"
tokio = { version = "1", features = ["sync"] }
tray-icon = { version = "=0.24.2", default-features = false }
''')
    (base / "src/lib.rs").write_text('''#![deny(warnings)]
use std::sync::Arc;
use tokio::sync::watch;
#[derive(Clone, Copy)]
pub enum Action { Open, Quit }
pub enum Event { Action(Action) }
pub struct Icon { rgba: Vec<u8>, size: u32 }
#[path = ''' + json.dumps(str(ROOT / "src/desktop_tray/native.rs")) + ''']
mod native;
pub fn check_native_adapter(rgba: Vec<u8>, size: u32, actions: watch::Sender<Option<Action>>) -> anyhow::Result<()> {
    native::initialize(Arc::new(Icon { rgba, size }), actions)
}
''')
    env = os.environ.copy()
    env.setdefault("CARGO_BUILD_JOBS", "4")
    subprocess.run(["cargo", "check", "--target", args.target], cwd=base, env=env, check=True)
    print(f"Exact native tray adapter checked for {args.target}; no OS execution or full-app link performed.")


if __name__ == "__main__":
    main()
