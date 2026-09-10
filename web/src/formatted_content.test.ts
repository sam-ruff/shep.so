import { it, expect, beforeAll } from "vitest";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { initSync, prepare_message } from "./wasm/shep_mail_content";
beforeAll(() =>
  initSync({
    module: new WebAssembly.Module(
      readFileSync(
        new URL("./wasm/shep_mail_content_bg.wasm", import.meta.url),
      ),
    ),
  }),
);
it("prepares the same confined HTML, inline WebP, plain alternative and CSP runtime in WASM", () => {
  const raw = readFileSync(
    new URL("../../shared/html-reader-fixture.eml", import.meta.url),
  );
  const prepared = JSON.parse(
    prepare_message(
      raw,
      JSON.stringify({ generation: "wasm-fixture", dark: true, quotes: false }),
    ),
  );
  expect(prepared.signature).toBe(
    createHash("sha256").update(raw).digest("hex"),
  );
  expect(prepared.text).toContain("Plain alternative");
  expect(
    prepared.remote_images.map((image: { url: string }) => image.url),
  ).toEqual(["https://images.example.test/news/banner.webp"]);
  expect(prepared.issues).toEqual([]);
  const runtime = readFileSync(
    new URL(
      "../../shared/mail-content/src/document/runtime.js",
      import.meta.url,
    ),
  );
  expect(prepared.document).toContain(
    `script-src 'sha256-${createHash("sha256").update(runtime).digest("base64")}'`,
  );
  const data = JSON.parse(
    prepared.document.match(
      /<script id="shep-data" type="application\/json">(.*?)<\/script>/s,
    )[1],
  );
  expect(Object.keys(data.images)).toHaveLength(1);
  expect(data.links).toEqual({ "0": "https://example.test/help" });
  expect(data.dark).toBe(true);
  for (const value of Object.values(data.images) as { bytes: string }[])
    expect(Buffer.from(value.bytes, "base64").subarray(8, 12).toString()).toBe(
      "WEBP",
    );
});
it("returns plain text without a frame and rejects invalid preparation options", () => {
  const bytes = new TextEncoder().encode(
    "Content-Type: text/plain\r\n\r\nStill text.",
  );
  expect(
    JSON.parse(prepare_message(bytes, JSON.stringify({ generation: "plain" }))),
  ).toMatchObject({
    text: "Still text.",
    document: null,
    remote_images: [],
    issues: [],
  });
  expect(() =>
    prepare_message(bytes, JSON.stringify({ generation: "" })),
  ).toThrow("generation");
});
