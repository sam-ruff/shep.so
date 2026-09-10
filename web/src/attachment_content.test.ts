import { it, expect, beforeAll } from "vitest";
import { readFileSync } from "node:fs";
import { initSync } from "./wasm/shep_mail_content";
import fixtures from "../../shared/attachment-fixtures.json";
import {
  catalogAttachments,
  readAttachment,
  filename,
} from "./attachment_content";
beforeAll(() =>
  initSync({
    module: new WebAssembly.Module(
      readFileSync(
        new URL("./wasm/shep_mail_content_bg.wasm", import.meta.url),
      ),
    ),
  }),
);
it("matches shared Rust attachment identities, filenames and exact binary bytes", async () => {
  for (const fixture of fixtures) {
    const raw = btoa(fixture.raw);
    const files = catalogAttachments(raw);
    expect(
      files.map((f) => ({
        ...f,
        bytes: btoa(String.fromCharCode(...readAttachment(raw, f.id))),
      })),
    ).toEqual(fixture.files);
  }
  const original = btoa(fixtures[0].raw);
  const changed = btoa(fixtures[0].raw.replace("AP8BDQo=", "AAECAwQ="));
  const oldId = catalogAttachments(original)[0].id;
  expect(catalogAttachments(changed)[0].id).not.toBe(oldId);
  expect(() => readAttachment(changed, oldId)).toThrow("changed");
});
it("limits MIME input and sanitizes sender-provided filenames", async () => {
  for (const [input, expected] of [
    ["../../file.txt", "file.txt"],
    ["C:\\temp\\evil.txt", "evil.txt"],
    ["\u202er\0ésumé.txt", "résumé.txt"],
    ["..", "attachment.bin"],
    ["/", "attachment.bin"],
  ])
    expect(filename(input)).toBe(expected);
  expect(() => catalogAttachments("! invalid base64")).toThrow();
  expect(() =>
    catalogAttachments("A".repeat(Math.ceil((25 * 1024 * 1024) / 3) * 4 + 4)),
  ).toThrow("25 MiB");
});
