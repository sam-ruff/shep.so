import { it, expect, beforeAll } from "vitest";
import { readFileSync } from "node:fs";
import { initSync, prepare_forward } from "./wasm/shep_mail_content";
import cases from "../../shared/forward-fixtures.json";

beforeAll(() =>
  initSync({
    module: new WebAssembly.Module(
      readFileSync(
        new URL("./wasm/shep_mail_content_bg.wasm", import.meta.url),
      ),
    ),
  }),
);

it("prepares the same complete forward, separate CID scopes and exact binary files as native Rust", () => {
  for (const fixture of cases) {
    const prepared = prepare_forward(new TextEncoder().encode(fixture.raw));
    try {
      const metadata = JSON.parse(prepared.metadata());
      expect(metadata.subject, fixture.name).toBe(fixture.subject);
      const retained =
        metadata.body +
        metadata.forward.html_head +
        metadata.forward.html_attributes +
        metadata.forward.html_body;
      for (const text of fixture.contains)
        expect(retained, fixture.name).toContain(text);
      for (const text of fixture.absent)
        expect(retained, fixture.name).not.toContain(text);
      expect(!!metadata.forward.html_body).toBe(fixture.html);
      expect(metadata.files).toEqual(
        fixture.files.map(({ hex: _hex, ...file }) => file),
      );
      fixture.files.forEach((file, index) => {
        expect(Buffer.from(prepared.file_bytes(index)).toString("hex")).toBe(
          file.hex,
        );
      });
      expect(() => prepared.file_bytes(fixture.files.length)).toThrow(
        "no attachment",
      );
      expect(() => prepared.file_bytes(0xffffffff)).toThrow("no attachment");
      // A bad index must not destroy an otherwise usable owned result.
      expect(JSON.parse(prepared.metadata())).toEqual(metadata);
    } finally {
      prepared.free();
    }
  }
});

it("rejects damaged resources and deep input without leaking source bytes or poisoning WASM", () => {
  const good = cases[0].raw;
  for (const broken of [
    good.replace("aW5saW5lIGZpeHR1cmU=", "PRIVATE-invalid***"),
    good.replace("AP8BDQo=", "PRIVATE-invalid***"),
  ]) {
    expect(() => prepare_forward(new TextEncoder().encode(broken))).toThrow(
      "Could not decode",
    );
    expect(() => prepare_forward(new TextEncoder().encode(broken))).not.toThrow(
      "PRIVATE",
    );
  }
  let deep = "";
  for (let i = 0; i < 4096; i++) {
    const boundary = `level${String(i).padStart(6, "0")}`;
    deep += `Content-Type: multipart/mixed; boundary=${boundary}\r\n\r\n--${boundary}\r\n`;
  }
  expect(() =>
    prepare_forward(new TextEncoder().encode(deep + "\r\nDeep")),
  ).toThrow("nested");
  const recovered = prepare_forward(
    new TextEncoder().encode("\r\nStill usable"),
  );
  try {
    expect(JSON.parse(recovered.metadata()).body).toContain("Still usable");
  } finally {
    recovered.free();
  }
});
