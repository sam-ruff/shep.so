// Reproducible native/browser MIME implementation. Generated glue stays ignored.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const local = path.join(root, "artifacts/wasm-tools/bin", process.platform === "win32" ? "wasm-bindgen.exe" : "wasm-bindgen");
const cli =
  process.env.SHEP_WASM_BINDGEN ?? (existsSync(local) ? local : "wasm-bindgen");
if (
  execFileSync(cli, ["--version"], { encoding: "utf8" }).trim() !==
  "wasm-bindgen 0.2.128"
) {
  throw new Error(
    "Install wasm-bindgen-cli 0.2.128 with cargo install --locked --root artifacts/wasm-tools --version 0.2.128 wasm-bindgen-cli",
  );
}
const target = path.join(root, "artifacts/mail-content-target");
const output = path.join(root, "web/src/wasm");
mkdirSync(output, { recursive: true });
execFileSync(
  "cargo",
  [
    "build",
    "--locked",
    "-p",
    "shep-mail-content",
    "--target",
    "wasm32-unknown-unknown",
    "--release",
  ],
  {
    cwd: root,
    env: { ...process.env, CARGO_TARGET_DIR: target },
    stdio: "inherit",
  },
);
execFileSync(
  cli,
  [
    path.join(target, "wasm32-unknown-unknown/release/shep_mail_content.wasm"),
    "--target",
    "web",
    "--out-dir",
    output,
  ],
  { cwd: root, stdio: "inherit" },
);
