import { it, expect, beforeAll } from "vitest";
import { readFileSync } from "node:fs";
import {
  initSync,
  message_body,
  attachment_catalog,
} from "./wasm/shep_mail_content";
import cases from "../../shared/reader-fixtures.json";

beforeAll(() =>
  initSync({
    module: new WebAssembly.Module(
      readFileSync(
        new URL("./wasm/shep_mail_content_bg.wasm", import.meta.url),
      ),
    ),
  }),
);
it("selects the same MIME alternatives, scoped CIDs and files as native Rust", () => {
  for (const fixture of cases) {
    const raw = new TextEncoder().encode(fixture.raw);
    expect(JSON.parse(message_body(raw)), fixture.name).toEqual(fixture.body);
    expect(
      JSON.parse(attachment_catalog(raw)).map(
        (file: { name: string }) => file.name,
      ),
      fixture.name,
    ).toEqual(fixture.files);
  }
});
it("refuses MIME nesting before a WASM stack trap and remains usable afterward", () => {
  let raw = "";
  for (let i = 0; i < 4096; i++) {
    const boundary = `level${String(i).padStart(6, "0")}`;
    raw += `Content-Type: multipart/mixed; boundary=${boundary}\r\n\r\n--${boundary}\r\n`;
  }
  raw += "Content-Type: text/plain\r\n\r\nDeep text";
  const bytes = new TextEncoder().encode(raw);
  expect(() => message_body(bytes)).toThrow("nested");
  expect(() => attachment_catalog(bytes)).toThrow("nested");
  expect(
    JSON.parse(message_body(new TextEncoder().encode("\r\nStill usable"))).text,
  ).toBe("Still usable");
});
