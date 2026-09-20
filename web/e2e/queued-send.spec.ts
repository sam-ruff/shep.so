import { test, expect, type Page } from "@playwright/test";
import { seed, profile } from "./mailbox-fixture";

async function compose(page: Page) {
  await page.getByRole("button", { name: "New message", exact: true }).click();
  const editor = page.getByRole("dialog", { name: "New message", exact: true });
  await editor.getByRole("combobox", { name: "From account", exact: true }).selectOption("work");
  await editor.getByRole("textbox", { name: "To", exact: true }).fill("recipient@example.test");
  await editor.getByRole("textbox", { name: "Subject", exact: true }).fill("Queued fixture message");
  await editor.getByRole("textbox", { name: "Message", exact: true }).fill("Exact saved fixture body");
  return editor;
}

test("offline Send closes into durable Outbox and can return to an editable draft without network", async ({ page }) => {
  await seed(page);
  let sends = 0;
  await page.route("**/api/mail/send", route => { sends++; return route.abort(); });
  const editor = await compose(page);
  await editor.getByRole("button", { name: "Send", exact: true }).click();
  await expect(editor).toHaveCount(0);
  await expect(page.getByText("Message queued in Outbox", { exact: true })).toBeVisible();
  await page.reload();
  await page.getByRole("button", { name: "Outbox", exact: true }).click();
  const outbox = page.getByRole("dialog", { name: "Outbox", exact: true });
  await expect(outbox).toContainText("Queued on this browser");
  await expect(outbox).toContainText("Queued fixture message");
  await page.screenshot({ path: "../artifacts/web/send-offline-outbox.png" });
  await outbox.getByRole("button", { name: "Return to drafts", exact: true }).click();
  await expect(outbox).toContainText("No outgoing messages need attention");
  await outbox.getByRole("button", { name: "Close", exact: true }).click();
  await page.getByRole("button", { name: "Drafts", exact: true }).click();
  await page.getByRole("button", { name: "Queued fixture message", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "Message", exact: true })).toHaveValue("Exact saved fixture body");
  expect(sends).toBe(0);
});

test("failed Send admission keeps the editor and its exact text", async ({ page }) => {
  await seed(page);
  const editor = await compose(page);
  await page.evaluate(() => {
    const put = IDBObjectStore.prototype.put;
    IDBObjectStore.prototype.put = function (...args: Parameters<IDBObjectStore["put"]>) {
      if (this.name === "outgoing") throw Error("Fixture queue admission failed");
      return put.apply(this, args);
    };
  });
  await editor.getByRole("button", { name: "Send", exact: true }).click();
  await expect(editor.getByRole("status")).toContainText("Could not save");
  await expect(editor.getByRole("textbox", { name: "Message", exact: true })).toHaveValue("Exact saved fixture body");
  const count = await page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const store = await BrowserStore.open(profile);
    const outgoing = await store.all("outgoing");
    store.close(); return outgoing.length;
  }, profile);
  expect(count).toBe(0);
});

test("Send closes the composer before a held provider reservation finishes", async ({ page }) => {
  await seed(page);
  await page.route("**/api/capabilities", route => route.fulfill({ json: { mail: true, endpoints: [] } }));
  await page.route("**/api/mail/probe", route => route.fulfill({ json: { connected: true } }));
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page.getByRole("button", { name: "Reconnect work@example.test", exact: true }).click();
  const account = page.getByRole("dialog", { name: "Reconnect work@example.test", exact: true });
  await account.getByLabel("Incoming password", { exact: true }).fill("fixture-password");
  await account.getByRole("button", { name: "Verify and save account", exact: true }).click();
  await expect(account).toHaveCount(0);
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  let held = false, release!: () => void;
  const pending = new Promise<void>(resolve => { release = resolve; });
  await page.route("**/api/mail/outgoing/reserve", async route => {
    held = true; await pending;
    await route.fulfill({ json: { id: "R".repeat(43), state: "reserved" } });
  });
  await page.route("**/api/mail/outgoing/prepare", route => route.abort());
  const editor = await compose(page);
  await editor.getByRole("button", { name: "Send", exact: true }).click();
  await expect.poll(() => held).toBe(true);
  await expect(editor).toHaveCount(0);
  await page.getByRole("button", { name: "Outbox", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Outbox", exact: true })).toContainText("Queued fixture message");
  await page.screenshot({ path: "../artifacts/web/send-reservation-held.png" });
  release();
});
