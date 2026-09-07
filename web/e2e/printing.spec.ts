import { test, expect, chromium, type Page } from "@playwright/test";
import {
  cpSync,
  readFileSync,
  mkdirSync,
  writeFileSync,
  readdirSync,
} from "node:fs";
import { resolve } from "node:path";
import { spawn, execFileSync } from "node:child_process";
const cases = JSON.parse(
  readFileSync(
    new URL("../../shared/forward-fixtures.json", import.meta.url),
    "utf8",
  ),
);
const profile = "F".repeat(43);
const image = readFileSync(
  new URL("../../assets/logo-light.webp", import.meta.url),
).toString("base64");
const raw = cases[0].raw
  .replace("aW5saW5lIGZpeHR1cmU=", image)
  .replace("Content-Type: image/png", "Content-Type: image/webp")
  .replace("<img ", '<img alt="Shep fixture logo" ');
const long = "Complete source. ".repeat(2500) + "\nEND OF COMPLETE ORIGINAL";

async function seed(page: Page) {
  await page.route("**/api/session", (r) =>
    r.fulfill({
      json: {
        email: "owner@example.test",
        csrf: "C".repeat(43),
        user_id: profile,
      },
    }),
  );
  await page.route("**/seed-print", (r) =>
    r.fulfill({
      contentType: "text/html",
      body: "<!doctype html><title>Print fixture setup</title>",
    }),
  );
  await page.goto("/seed-print");
  await page.evaluate(
    async ({ profile, raw, long }) => {
      const { BrowserStore } = await import("/src/storage.ts");
      const store = await BrowserStore.open(profile);
      const account = {
        id: "fixture",
        name: "Forward fixture",
        email: "owner@example.test",
        protocol: "Pop3",
        host: "mail.example.test",
        port: 995,
        username: "fixture",
        incoming_security: "Tls",
        incoming_auth: "Password",
        smtp_host: "mail.example.test",
        smtp_port: 465,
        smtp_username: "fixture",
        smtp_security: "Tls",
        smtp_auth: "Automatic",
        smtp_separate_password: false,
        sent_copy: "LocalOnly",
        sent_folder: "Sent",
      };
      const changes: any[] = [
        { store: "accounts", key: account.id, value: account },
      ];
      const rows = [
        ["source", "Café project", "Complete café.", raw],
        [
          "long",
          "Long forward source",
          "Short cached preview.",
          `Subject: Long forward source\r\n\r\n${long}`,
        ],
        [
          "broken",
          "Damaged forward",
          "Readable damaged source.",
          raw.replace(
            /Content-Transfer-Encoding: base64\r\n\r\n[^\r]+/,
            "Content-Transfer-Encoding: base64\r\n\r\nPRIVATE-invalid***",
          ),
        ],
        [
          "other",
          "Other letter",
          "Another message.",
          "Subject: Other letter\r\n\r\nAnother message.",
        ],
      ];
      for (const [i, [id, subject, text, mime]] of rows.entries()) {
        const core = {
          id,
          account_id: account.id,
          remote_id: id,
          folder: "INBOX",
          sender: "Sender <sender@example.test>",
          recipient: account.email,
          subject,
          preview: text,
          timestamp: 1788692400 - i,
          unread: false,
          starred: false,
          attachment_count: id === "source" ? 2 : 0,
        };
        changes.push(
          { store: "mail", key: id, value: { core, text } },
          {
            store: "raw",
            key: id,
            value: btoa(String.fromCharCode(...new TextEncoder().encode(mime))),
          },
        );
      }
      await store.commit(changes);
      store.close();
    },
    { profile, raw, long },
  );
  await page.goto("/");
}

const open = (page: Page, name = "Café project") =>
  page.locator(".mail-row").getByRole("button", { name, exact: true }).click();
async function print(page: Page) {
  const popup = page.waitForEvent("popup");
  await page.getByRole("button", { name: "Print", exact: true }).click();
  return popup;
}
test("print preview retains full formatted source, inline image and quoted history after navigation", async ({
  page,
}) => {
  await seed(page);
  await open(page);
  const preview = await print(page);
  await expect(
    preview.getByRole("button", { name: "Print", exact: true }),
  ).toBeEnabled();
  await open(page, "Other letter");
  const frame = preview.frameLocator("iframe");
  await expect(frame.locator("body")).toContainText("Complete café.");
  await expect(frame.locator("img").first()).toBeVisible();
  expect(
    await frame
      .locator("img")
      .first()
      .evaluate(
        (img: HTMLImageElement) => img.complete && img.naturalWidth > 0,
      ),
  ).toBe(true);
  expect(
    await frame
      .locator("body")
      .evaluate((node) => getComputedStyle(node).backgroundColor),
  ).toBe("rgb(255, 255, 255)");
  await preview.screenshot({ path: "../artifacts/web/print-formatted.png" });
  await preview.close();
  await expect(
    page.getByRole("region", { name: "Message reader" }),
  ).toContainText("Other letter");
});
test("printing reports damaged content, retries and uses complete plain text", async ({
  page,
}) => {
  await seed(page);
  await open(page, "Damaged forward");
  const failed = await print(page);
  await expect(failed.getByRole("button", { name: "Retry" })).toBeVisible();
  await failed.getByRole("button", { name: "Retry" }).click();
  await expect(failed.getByRole("button", { name: "Retry" })).toBeVisible();
  await failed.close();
  await page
    .getByRole("combobox", { name: "Message format" })
    .selectOption("Plain text");
  const plain = await print(page);
  await expect(
    plain.getByRole("button", { name: "Print", exact: true }),
  ).toBeEnabled();
  await expect(plain.frameLocator("iframe").locator("body")).toContainText(
    "Complete café.",
  );
  await plain.close();
  await open(page, "Long forward source");
  const long = await print(page);
  await expect(
    long.getByRole("button", { name: "Print", exact: true }),
  ).toBeEnabled();
  await expect(long.frameLocator("iframe").locator("body")).toContainText(
    "END OF COMPLETE ORIGINAL",
  );
  await long.close();
});
test("owned browser printer creates a paginated PDF through window.print", async ({}, testInfo) => {
  test.setTimeout(90000);
  const dir = resolve(testInfo.outputPath("native-print"));
  const profile = resolve(dir, "profile"),
    output = resolve(dir, "pdf");
  mkdirSync(resolve(profile, "Default"), { recursive: true });
  mkdirSync(output, { recursive: true });
  writeFileSync(
    resolve(profile, "Default", "Preferences"),
    JSON.stringify({
      printing: {
        print_preview_sticky_settings: {
          appState: JSON.stringify({
            version: 2,
            recentDestinations: [
              { id: "Save as PDF", origin: "local", account: "" },
            ],
            selectedDestinationId: "Save as PDF",
            isHeaderFooterEnabled: false,
            isCssBackgroundEnabled: true,
          }),
        },
      },
      savefile: { default_directory: output },
      download: { default_directory: output },
    }),
  );
  const xvfb = spawn(
    "Xvfb",
    ["-displayfd", "1", "-screen", "0", "1440x1000x24", "-nolisten", "tcp"],
    { stdio: ["ignore", "pipe", "ignore"] },
  );
  try {
    const display = await new Promise<string>((done, reject) => {
      xvfb.stdout.once("data", (data) => done(":" + data.toString().trim()));
      xvfb.once("error", reject);
      xvfb.once("exit", () => reject(new Error("Owned display stopped")));
    });
    const browser = await chromium.launchPersistentContext(profile, {
      headless: false,
      env: { ...process.env, DISPLAY: display },
      args: ["--ozone-platform=x11", "--kiosk-printing"],
      baseURL: "http://127.0.0.1:5180",
      viewport: { width: 1440, height: 920 },
    });
    try {
      const page = await browser.newPage();
      await seed(page);
      await open(page);
      const preview = await print(page);
      await expect
        .poll(
          () => readdirSync(output).filter((p) => p.endsWith(".pdf")).length,
          { timeout: 30000 },
        )
        .toBe(1);
      const pdf = resolve(
        output,
        readdirSync(output).find((p) => p.endsWith(".pdf"))!,
      );
      const text = execFileSync("pdftotext", [pdf, "-"], { encoding: "utf8" });
      expect(text).toContain("Café project");
      expect(text).toContain("sender@example.test");
      expect(text).toContain("duplicate.bin");
      expect(text).toContain("Complete café.");
      expect(text).not.toContain("private@example.test");
      execFileSync("pdftoppm", [
        "-f",
        "1",
        "-singlefile",
        "-scale-to",
        "1200",
        "-png",
        pdf,
        resolve(dir, "formatted"),
      ]);
      await preview.close();
      await open(page, "Long forward source");
      const longPreview = await print(page);
      await expect
        .poll(
          () => readdirSync(output).filter((p) => p.endsWith(".pdf")).length,
          { timeout: 30000 },
        )
        .toBe(2);
      const longPdf = resolve(
        output,
        readdirSync(output).find(
          (p) => p.endsWith(".pdf") && resolve(output, p) !== pdf,
        )!,
      );
      expect(
        execFileSync("pdftotext", [longPdf, "-"], { encoding: "utf8" }),
      ).toContain("END OF COMPLETE ORIGINAL");
      expect(
        Number(
          /Pages:\s+(\d+)/.exec(
            execFileSync("pdfinfo", [longPdf], { encoding: "utf8" }),
          )![1],
        ),
      ).toBeGreaterThan(1);
      await longPreview.close();
      const evidence = resolve("../artifacts/web/print-output");
      mkdirSync(evidence, { recursive: true });
      cpSync(output, resolve(evidence, "pdf"), { recursive: true });
      cpSync(resolve(dir, "formatted.png"), resolve(evidence, "formatted.png"));
    } finally {
      await browser.close();
    }
  } finally {
    xvfb.kill();
  }
});

test("held print preparation leaves navigation and a newer composer usable", async ({
  page,
  context,
}) => {
  await seed(page);
  await open(page);
  let release!: () => void;
  const held = new Promise<void>((resolve) => (release = resolve));
  await context.route("**/src/printing_worker.ts*", async (route) => {
    await held;
    await route.continue().catch(() => {});
  });
  try {
    const preview = await print(page);
    await expect(
      page.getByRole("button", { name: "Preparing print…", exact: true }),
    ).toBeDisabled();
    await open(page, "Other letter");
    await page
      .getByRole("button", { name: "New message", exact: true })
      .click();
    const editor = page.getByRole("dialog", { name: "New message" });
    await editor
      .getByLabel("Subject", { exact: true })
      .fill("Independent print note");
    release();
    await expect(
      preview.getByRole("button", { name: "Print", exact: true }),
    ).toBeEnabled();
    await expect(preview.frameLocator("iframe").locator("body")).toContainText(
      "Complete café.",
    );
    await expect(editor.getByLabel("Subject", { exact: true })).toHaveValue(
      "Independent print note",
    );
    await preview.close();
  } finally {
    release();
  }
});
test("Print can be remapped and disabled and does not run in editable fields", async ({
  page,
  context,
}) => {
  await seed(page);
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page.getByLabel("Theme", { exact: true }).selectOption("dark");
  await page.getByRole("button", { name: "Remap print", exact: true }).click();
  await page.keyboard.press("Alt+p");
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await open(page);
  const mailBody = page
    .frameLocator('iframe[title="Formatted message"]')
    .locator("body");
  await expect(mailBody).toContainText("Complete café.");
  expect(
    await mailBody.evaluate((node) => getComputedStyle(node).backgroundColor),
  ).toBe("rgb(255, 255, 255)");
  expect(await mailBody.evaluate((node) => getComputedStyle(node).color)).toBe(
    "rgb(24, 24, 27)",
  );
  await page.screenshot({
    path: "../artifacts/web/reader-authored-background-dark.png",
  });
  const pending = page.waitForEvent("popup");
  await page.keyboard.press("Alt+p");
  const preview = await pending;
  await expect(
    preview.getByRole("button", { name: "Print", exact: true }),
  ).toBeEnabled();
  await preview.setViewportSize({ width: 900, height: 640 });
  await preview.screenshot({ path: "../artifacts/web/print-dark-compact.png" });
  await preview.close();
  await page.getByRole("button", { name: "New message", exact: true }).click();
  await page
    .getByRole("dialog")
    .getByLabel("Subject", { exact: true })
    .fill("p");
  await page.keyboard.press("Alt+p");
  expect(context.pages()).toHaveLength(1);
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Save draft", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page.getByRole("button", { name: "Clear print", exact: true }).click();
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await open(page);
  await page.keyboard.press("Alt+p");
  expect(context.pages()).toHaveLength(1);
});
