import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import AxeBuilder from "@axe-core/playwright";
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
  await page.route("**/seed-forward", (r) =>
    r.fulfill({
      contentType: "text/html",
      body: "<!doctype html><title>Forward fixture setup</title>",
    }),
  );
  await page.goto("/seed-forward");
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
async function drafts(page: Page) {
  return page.evaluate(async (profile) => {
    const { BrowserStore } = await import("/src/storage.ts");
    const store = await BrowserStore.open(profile);
    try {
      return await store.snapshot(["drafts", "draftFiles", "outgoing"]);
    } finally {
      store.close();
    }
  }, profile);
}
const open = (page: Page, name = "Café project") =>
  page.locator(".mail-row").getByRole("button", { name, exact: true }).click();
test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 920 });
  await seed(page);
});

test("Forward keeps exact files and complete text through autosave, removal, restart and Send refusal", async ({
  page,
}) => {
  await open(page);
  await page.getByRole("button", { name: "Forward", exact: true }).click();
  let dialog = page.getByRole("dialog", { name: "Forward message" });
  await expect(dialog.getByLabel("To", { exact: true })).toBeEditable();
  for (const label of ["To", "Cc", "Bcc"])
    await expect(dialog.getByLabel(label, { exact: true })).toHaveValue("");
  await expect(dialog.getByLabel("Subject", { exact: true })).toHaveValue(
    "Fwd: Café project",
  );
  const body = await dialog.getByLabel("Message", { exact: true }).inputValue();
  expect(body).toContain("Complete café.");
  expect(body).not.toContain("private@example.test");
  await expect(dialog.locator(".draft-file")).toHaveCount(3);
  await dialog.getByLabel("To", { exact: true }).fill("reviewer@example.test");
  await dialog
    .getByLabel("Message", { exact: true })
    .fill("Please review <this>.\n" + body);
  await dialog
    .getByRole("button", { name: "Remove duplicate.bin", exact: true })
    .first()
    .click();
  await expect(dialog.locator(".draft-file")).toHaveCount(2);
  await expect(dialog.getByLabel("To", { exact: true })).toBeEditable();
  const before: any = await drafts(page);
  expect(before.drafts[0].forward.html_body).toContain("<table>");
  expect(before.drafts[0].forward.html_body).not.toContain("<script");
  expect(
    before.draftFiles.find((f: any) => f.info.content_id)?.info.content_id,
  ).toMatch(/^shep-/);
  expect(
    await new AxeBuilder({ page })
      .include("dialog[open]")
      .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
      .analyze(),
  ).toMatchObject({ violations: [] });
  await page.screenshot({ path: "../artifacts/web/forward-light.png" });
  await dialog.getByRole("button", { name: "Save draft", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await page.reload();
  await page.getByRole("button", { name: "Drafts", exact: true }).click();
  await page
    .getByRole("button", { name: "Fwd: Café project", exact: true })
    .click();
  dialog = page.getByRole("dialog", { name: "Forward message" });
  await expect(dialog.getByLabel("Message")).toHaveValue(
    "Please review <this>.\n" + body,
  );
  await expect(dialog.locator(".draft-file")).toHaveCount(2);
  const after: any = await drafts(page);
  expect(after.drafts[0].forward).toEqual(before.drafts[0].forward);
  expect(after.draftFiles.map((f: any) => f.info)).toEqual(
    before.draftFiles.map((f: any) => f.info),
  );
  await dialog.getByRole("button", { name: "Send", exact: true }).click();
  await expect(dialog.getByRole("status")).toContainText("Reconnect");
  await expect(dialog.getByLabel("Message")).toBeEditable();
  await dialog.getByRole("button", { name: "Save draft", exact: true }).click();
  await page.getByRole("button", { name: "Inbox", exact: true }).click();
  await open(page, "Long forward source");
  await page.getByRole("button", { name: "Forward", exact: true }).click();
  dialog = page.getByRole("dialog", { name: "Forward message" });
  await expect(dialog.getByLabel("Message")).toHaveValue(
    new RegExp("END OF COMPLETE ORIGINAL$"),
  );
  expect(
    (await dialog.getByLabel("Message").inputValue()).length,
  ).toBeGreaterThan(32000);
});

test("worker failure and damaged resources stay retryable; a lost commit response never creates duplicate drafts", async ({
  page,
  context,
}) => {
  await open(page);
  const route = "**/src/forward_worker.ts*";
  await context.route(route, (r) => r.abort("failed"));
  await page.getByRole("button", { name: "Forward", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Forward preparation stopped",
  );
  expect((await drafts(page)).drafts).toHaveLength(0);
  await context.unroute(route);
  // Object-scoped response loss wraps the real durable operation; all actions
  // still come from the visible Forward control.
  await page.evaluate(async () => {
    const { GatewayRepository } = await import("/src/provider.ts");
    const original = GatewayRepository.prototype.forward;
    let first = true;
    GatewayRepository.prototype.forward = async function (...args: any[]) {
      const result = await original.apply(this, args);
      if (first) {
        first = false;
        throw new Error(
          "Synthetic lost forward acknowledgment. Retry Forward.",
        );
      }
      return result;
    };
  });
  await page.getByRole("button", { name: "Forward", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText(
    "lost forward acknowledgment",
  );
  const committed: any = await drafts(page);
  expect(committed.drafts).toHaveLength(1);
  await page.getByRole("button", { name: "Forward", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Forward message" });
  await expect(dialog.getByLabel("To", { exact: true })).toBeEditable();
  expect((await drafts(page)).drafts).toEqual(committed.drafts);
  await dialog.getByRole("button", { name: "Save draft", exact: true }).click();
  await open(page, "Damaged forward");
  await page.getByRole("button", { name: "Forward", exact: true }).click();
  await expect(
    page.getByRole("alert").filter({ hasText: "Could not decode" }),
  ).toContainText("Could not decode");
  expect((await drafts(page)).drafts).toHaveLength(1);
  await page.screenshot({ path: "../artifacts/web/forward-retry.png" });
});

test("pending Forward preserves navigation and a newer editor", async ({
  page,
  context,
}) => {
  await open(page, "Other letter");
  let release!: () => void;
  const held = new Promise<void>((r) => (release = r));
  await context.route("**/src/forward_worker.ts*", async (r) => {
    await held;
    await r.continue();
  });
  await page.getByRole("button", { name: "Forward", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Preparing forward…", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Inbox", exact: true }).click();
  await open(page, "Long forward source");
  await page.getByRole("button", { name: "New message", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "New message" });
  await expect(dialog.getByLabel("To", { exact: true })).toBeEditable();
  await dialog.getByLabel("Subject", { exact: true }).fill("Independent note");
  release();
  await expect
    .poll(async () =>
      ((await drafts(page)).drafts as any[]).some(
        (d) => d.subject === "Fwd: Other letter",
      ),
    )
    .toBe(true);
  await expect(dialog).toBeVisible();
  await expect(dialog.getByLabel("Subject", { exact: true })).toHaveValue(
    "Independent note",
  );
  await page.screenshot({ path: "../artifacts/web/forward-independent.png" });
});

test("Forward shortcut can be remapped and disabled without acting in text fields; compact dark layout remains usable", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page.getByLabel("Theme", { exact: true }).selectOption("dark");
  const shortcut = page.getByRole("button", {
    name: "Remap forward",
    exact: true,
  });
  await shortcut.click();
  await page.keyboard.press("Alt+f");
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await open(page);
  await page.keyboard.press("Alt+f");
  const dialog = page.getByRole("dialog", { name: "Forward message" });
  await expect(dialog).toBeVisible();
  await page.setViewportSize({ width: 900, height: 640 });
  await dialog.getByLabel("To", { exact: true }).fill("f@example.test");
  await page.keyboard.press("Alt+f");
  expect((await drafts(page)).drafts).toHaveLength(1);
  await expect(dialog.locator(".draft-file")).toHaveCount(3);
  expect(
    await new AxeBuilder({ page })
      .include("dialog[open]")
      .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
      .analyze(),
  ).toMatchObject({ violations: [] });
  await page.screenshot({ path: "../artifacts/web/forward-dark-compact.png" });
  await dialog.locator(".draft-file").last().scrollIntoViewIfNeeded();
  await expect(dialog.locator(".draft-file").last()).toBeInViewport();
  await page.screenshot({ path: "../artifacts/web/forward-dark-files.png" });
  await dialog.getByRole("button", { name: "Save draft", exact: true }).click();
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page
    .getByRole("button", { name: "Clear forward", exact: true })
    .click();
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await open(page);
  await page.keyboard.press("Alt+f");
  await expect(page.getByRole("dialog")).toHaveCount(0);
});
