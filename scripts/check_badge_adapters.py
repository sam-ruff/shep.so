#!/usr/bin/env python3
"""Check exact badge adapters for platform API compatibility, without app linking.

Actual Windows/macOS native execution remains a separate release requirement.
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
    base = ROOT / "artifacts" / "badge-adapter-check"
    (base / "src").mkdir(parents=True, exist_ok=True)
    (base / "Cargo.toml").write_text('''[package]
name = "shep-native-badge-adapter-check"
version = "0.0.0"
edition = "2024"
publish = false
[workspace]
[dependencies]
anyhow = "1"
tokio = { version = "1", features = ["sync"] }
tracing = "0.1"
[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "=0.62.2", features = ["Win32_UI_Shell", "Win32_UI_WindowsAndMessaging", "Win32_System_Com"] }
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "=0.6.4"
objc2-app-kit = { version = "=0.3.2", default-features = false, features = ["NSApplication", "NSDockTile", "NSResponder"] }
objc2-foundation = { version = "=0.3.2", default-features = false, features = ["NSString"] }
dispatch2 = "=0.3.1"
''')
    source = '#![deny(warnings)]\n'
    if args.target.startswith("aarch64-apple"):
        for module in ["count_delivery", "macos"]:
            source += f'#[path = {json.dumps(str(ROOT / "src/desktop_badge" / (module + ".rs")))}]\nmod {module};\n'
        source += 'pub async fn check(counts: tokio::sync::watch::Receiver<u64>) { macos::run(counts).await }\n'
    else:
        source += '''mod overlay {
    pub const SIZE: usize = 32;
    pub struct Frame { pub count: u64, pub rgba: Vec<u8>, pub description: String }
}
'''
        source += f'#[path = {json.dumps(str(ROOT / "src/desktop_badge/windows.rs"))}]\nmod windows;\n'
        source += 'pub fn check(window: isize, frame: std::sync::Arc<overlay::Frame>) -> anyhow::Result<()> { windows::apply(window, frame) }\n'
    (base / "src/lib.rs").write_text(source)
    env = os.environ.copy()
    env.setdefault("CARGO_BUILD_JOBS", "4")
    subprocess.run(["cargo", "check", "--target", args.target], cwd=base, env=env, check=True)
    print(f"Exact badge adapter checked for {args.target}; no OS execution or full-app link performed.")


if __name__ == "__main__":
    main()
