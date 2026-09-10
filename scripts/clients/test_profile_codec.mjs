// Exercise the actual Rust WASM entry against the same native contract fixtures.
// No browser profile, provider credentials or persistent storage is opened.
import { execFileSync } from "node:child_process";
import { readFileSync, existsSync, mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const local = path.join(
  root,
  "artifacts/wasm-tools/bin",
  process.platform === "win32" ? "wasm-bindgen.exe" : "wasm-bindgen",
);
const cli =
  process.env.SHEP_WASM_BINDGEN ?? (existsSync(local) ? local : "wasm-bindgen");
assert.equal(
  execFileSync(cli, ["--version"], { encoding: "utf8" }).trim(),
  "wasm-bindgen 0.2.128",
);
const target = path.join(root, "artifacts/profile-codec-target");
const output = path.join(root, "artifacts/profile-codec-wasm");
mkdirSync(output, { recursive: true });
execFileSync(
  "cargo",
  [
    "build",
    "--locked",
    "-p",
    "shep-profile-core",
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
    path.join(target, "wasm32-unknown-unknown/release/shep_profile_core.wasm"),
    "--target",
    "nodejs",
    "--out-dir",
    output,
  ],
  { stdio: "inherit" },
);
const { validate_profile_operation } = createRequire(import.meta.url)(
  path.join(output, "shep_profile_core.js"),
);
const golden = JSON.parse(
  readFileSync(path.join(root, "shared/profile-operation.json"), "utf8"),
);
const { cases } = JSON.parse(
  readFileSync(path.join(root, "shared/profile-cases.json"), "utf8"),
);
const messages = {
  Invalid:
    "This profile record is invalid. Keep the local setup and retry discovery.",
  Upgrade:
    "This profile uses an unsupported version or capability. Update Shep before syncing it.",
  LocalData:
    "Device state or credentials cannot be included in this profile metadata record.",
};
const check = (input) =>
  new TextDecoder().decode(
    validate_profile_operation(new TextEncoder().encode(input)),
  );
for (const test of cases) {
  const value = structuredClone(golden);
  for (const patch of test.patches) {
    const keys = patch.path.slice(1).split("/");
    let target = value;
    for (const key of keys.slice(0, -1)) target = target[key];
    target[keys.at(-1)] = patch.value;
  }
  if (test.error)
    assert.throws(
      () => check(JSON.stringify(value)),
      { message: messages[test.error] },
      test.name,
    );
  else
    assert.deepEqual(
      JSON.parse(check(JSON.stringify(value))),
      value,
      test.name,
    );
}
assert.throws(
  () =>
    check(JSON.stringify(golden).replace('"major":1', '"major":2,"major":1')),
  { message: messages.Invalid },
);
assert.throws(() => check(" ".repeat(1024 * 1024 + 1)), /record size/);
assert.throws(() => check("[".repeat(150) + "0" + "]".repeat(150)), {
  message: messages.Invalid,
});
console.log(
  `${cases.length} shared profile fixtures and duplicate/size/depth rejection pass in Rust WASM.`,
);
