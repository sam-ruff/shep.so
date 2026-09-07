import { it, expect, beforeAll } from "vitest";
import { readFileSync } from "node:fs";
import { initSync } from "./wasm/shep_mail_content";
import { preparePrint } from "./printing_content";
beforeAll(() =>
  initSync({
    module: new WebAssembly.Module(
      readFileSync(
        new URL("./wasm/shep_mail_content_bg.wasm", import.meta.url),
      ),
    ),
  }),
);
it("prints complete raw source with confined resources and excludes Bcc", async () => {
  const raw = readFileSync(
    new URL("../../shared/html-reader-fixture.eml", import.meta.url),
    "utf8",
  );
  const value = await preparePrint(Buffer.from(raw).toString("base64"), {
    generation: "wasm-print",
    plain: false,
  });
  expect(value.document).toContain("Alpha in quoted history.");
  expect(value.document).toContain("urn:shep-image:");
  expect(value.document).not.toContain("images.example.test");
  const text = "Complete line. ".repeat(3000) + "END OF PRINT";
  const plain = await preparePrint(
    Buffer.from(
      `Subject: Whole message\r\nBcc: private@example.test\r\n\r\n${text}`,
    ).toString("base64"),
    { generation: "plain-print", plain: true },
  );
  expect(plain.document).toContain(text);
  expect(plain.document).not.toContain("private@example.test");
});
