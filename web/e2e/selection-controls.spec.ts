import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
const profile = "C".repeat(43);
const subject = (i: number) =>
  `Selection letter ${i.toString().padStart(3, "0")}`;
async function seed(page: Page) {
  await page.route("**/api/session", (r) =>
    r.fulfill({
      json: {
        email: "owner@example.test",
        user_id: profile,
        csrf: "X".repeat(43),
      },
    }),
  );
  await page.route("**/seed-selection", (r) =>
    r.fulfill({
      contentType: "text/html",
      body: "<!doctype html><title>Selection fixture setup</title>",
    }),
  );
  await page.goto("/seed-selection");
  await page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    const changes: any[] = ["work", "personal"].map((id) => ({
      store: "accounts",
      key: id,
      value: {
        id,
        name: id,
        email: `${id}@example.test`,
        protocol: "Pop3",
        host: "mail.example.test",
        port: 995,
        username: id,
        incoming_security: "Tls",
        incoming_auth: "Password",
        smtp_host: "mail.example.test",
        smtp_port: 465,
        smtp_username: id,
        smtp_security: "Tls",
        smtp_auth: "Automatic",
        smtp_separate_password: false,
        sent_copy: "LocalOnly",
        sent_folder: "Sent",
      },
    }));
    for (let i = 0; i < 125; i++) {
      const id = `m${i.toString().padStart(3, "0")}`,
        text = `Body for selection letter ${i}.`;
      const core = {
        id,
        account_id: i % 2 ? "personal" : "work",
        remote_id: id,
        folder: "INBOX",
        sender: "Sender <sender@example.test>",
        recipient: "work@example.test",
        subject: `Selection letter ${i.toString().padStart(3, "0")}`,
        preview: text,
        timestamp: 1788692400 - i,
        unread: true,
        starred: false,
        attachment_count: 0,
      };
      changes.push(
        { store: "mail", key: id, value: { core, text } },
        {
          store: "raw",
          key: id,
          value: btoa(
            `Subject: ${core.subject}\r\nContent-Type: text/plain\r\n\r\n${text}`,
          ),
        },
      );
    }
    await store.commit(changes);
    store.close();
  }, profile);
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: subject(0), exact: true }),
  ).toBeVisible();
}
const status = (page: Page) =>
  page.getByRole("status", { name: "Selection status" });
async function count(page: Page, value: number) {
  await expect(status(page)).toContainText(`${value} selected`);
  await expect(status(page)).toHaveAttribute("data-pending", "false");
}
const checkbox = (page: Page, i: number) =>
  page.getByRole("checkbox", { name: `Select ${subject(i)}`, exact: true });
async function unread(page: Page) {
  return page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    const rows = await store.all("mail");
    store.close();
    return rows.filter((m: any) => m.core.unread).length;
  }, profile);
}

test("checkboxes and cross-page ranges keep full membership without reading mail", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 920 });
  await seed(page);
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await count(page, 0);
  await checkbox(page, 0).click();
  await count(page, 1);
  await page.getByRole("button", { name: "Next page", exact: true }).click();
  await page.getByRole("button", { name: "Next page", exact: true }).click();
  await expect(checkbox(page, 100)).toBeVisible();
  await checkbox(page, 100).click({ modifiers: ["Shift"] });
  await count(page, 101);
  await expect(checkbox(page, 100)).toBeChecked();
  await expect(checkbox(page, 101)).not.toBeChecked();
  expect(await unread(page)).toBe(125);
  await page
    .getByRole("button", { name: "Clear selection", exact: true })
    .click();
  await count(page, 0);
  await expect(checkbox(page, 100)).not.toBeChecked();
  await page
    .getByRole("button", { name: "Select all messages", exact: true })
    .click();
  await count(page, 125);
  await page
    .getByRole("button", { name: "Previous page", exact: true })
    .click();
  await expect(checkbox(page, 50)).toBeChecked();
  await page.screenshot({
    path: "../artifacts/web/selection-cross-page-light.png",
  });
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await expect(page.getByRole("checkbox")).toHaveCount(0);
  expect(await unread(page)).toBe(125);
});

test("modifier selection, keyboard navigation and Select all stay scoped to the message list", async ({
  page,
}) => {
  await seed(page);
  await page
    .getByRole("button", { name: subject(0), exact: true })
    .click({ modifiers: ["Control"] });
  await count(page, 1);
  await page
    .getByRole("button", { name: subject(1), exact: true })
    .click({ modifiers: ["Control"] });
  await count(page, 2);
  await page
    .getByRole("button", { name: subject(3), exact: true })
    .click({ modifiers: ["Shift"] });
  await count(page, 3);
  await page.keyboard.press("ArrowDown");
  await expect(checkbox(page, 4)).toBeFocused();
  await page.keyboard.press("Space");
  await count(page, 4);
  await page.keyboard.press("Control+a");
  await count(page, 125);
  // Message-target shortcuts cannot accidentally act on the retained reader.
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Control+d");
  await count(page, 125);
  expect(await unread(page)).toBe(125);
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("button", { name: "Select", exact: true }),
  ).toBeVisible();
  const search = page.getByRole("textbox", {
    name: "Search conversations",
    exact: true,
  });
  await search.fill("Selection");
  await search.press("Control+a");
  expect(
    await search.evaluate(
      (node: HTMLInputElement) => node.selectionEnd! - node.selectionStart!,
    ),
  ).toBe(9);
  await expect(
    page.getByRole("button", { name: "Done", exact: true }),
  ).toHaveCount(0);
  await search.fill("");
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await page.keyboard.press("Control+a");
  await expect(
    page.getByRole("button", { name: "Done", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: subject(0), exact: true }).click();
  await page.getByLabel("Message reader", { exact: true }).click();
  await page.keyboard.press("Control+a");
  await expect(
    page.getByRole("button", { name: "Done", exact: true }),
  ).toHaveCount(0);
});

test("scope resets immediately while passive arrivals wait for explicit recapture", async ({
  page,
}) => {
  await seed(page);
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page
    .getByRole("button", { name: "Select all messages", exact: true })
    .click();
  await count(page, 125);
  await page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    const current = await store.get("mail", "m000");
    current.core = {
      ...current.core,
      id: "arrival",
      remote_id: "arrival",
      subject: "New selection arrival",
      timestamp: 1788692401,
    };
    await store.commit([{ store: "mail", key: "arrival", value: current }]);
    store.close();
  }, profile);
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(
    page.getByRole("checkbox", {
      name: "Select New selection arrival",
      exact: true,
    }),
  ).toBeVisible();
  await count(page, 125);
  await expect(
    page.getByRole("checkbox", {
      name: "Select New selection arrival",
      exact: true,
    }),
  ).not.toBeChecked();
  await page
    .getByRole("button", { name: "Select all messages", exact: true })
    .click();
  await count(page, 126);
  await page
    .getByRole("combobox", { name: "Sort", exact: true })
    .selectOption("Oldest first");
  await expect(
    page.getByRole("button", { name: "Done", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Search conversations", exact: true })
    .pressSequentially("Nothing matches", { delay: 20 });
  await expect(
    page.getByRole("textbox", { name: "Search conversations", exact: true }),
  ).toHaveValue("Nothing matches");
  await expect(
    page.getByRole("button", { name: "Done", exact: true }),
  ).toHaveCount(0);
});

test("startup failure offers Retry and compact dark selection stays accessible", async ({
  page,
}) => {
  await seed(page);
  let fail = true;
  await page.route("**/src/selection_worker.ts*", async (route) => {
    if (fail) {
      fail = false;
      await route.abort();
    } else await route.continue();
  });
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await expect(
    page.getByRole("alert", { name: "Selection error" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Retry selection", exact: true })
    .click();
  await count(page, 0);
  await page
    .getByRole("button", { name: "Select all messages", exact: true })
    .click();
  await count(page, 125);
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page
    .getByRole("combobox", { name: "Theme", exact: true })
    .selectOption("dark");
  await page.setViewportSize({ width: 900, height: 640 });
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await count(page, 125);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({
    path: "../artifacts/web/selection-dark-compact.png",
  });
});

test("Select all can be remapped and disabled without taking text shortcuts", async ({
  page,
}) => {
  await seed(page);
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page
    .getByRole("button", { name: "Remap select all messages", exact: true })
    .click();
  await page.keyboard.press("Control+Shift+a");
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await page.locator(".rows").focus();
  await page.keyboard.press("Control+a");
  await expect(
    page.getByRole("button", { name: "Done", exact: true }),
  ).toHaveCount(0);
  await page.keyboard.press("Control+Shift+a");
  await count(page, 125);
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page
    .getByRole("button", { name: "Clear select all messages", exact: true })
    .click();
  await page.reload();
  await page.locator(".rows").focus();
  await page.keyboard.press("Control+Shift+a");
  await expect(
    page.getByRole("button", { name: "Done", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page
    .getByRole("button", { name: "Select all messages", exact: true })
    .click();
  await count(page, 125);
});

test("pending capture leaves paging and preferences usable and cannot restore an abandoned scope", async ({
  page,
}) => {
  await seed(page);
  let release!: () => void,
    requested = false;
  const gate = new Promise<void>((resolve) => (release = resolve));
  await page.route("**/sqlite3.wasm*", async (route) => {
    requested = true;
    await gate;
    await route.continue();
  });
  try {
    await page.getByRole("button", { name: "Select", exact: true }).click();
    await expect.poll(() => requested).toBe(true);
    await page
      .getByRole("button", { name: "Select all messages", exact: true })
      .click();
    await expect(status(page)).toContainText("125 selected");
    await expect(status(page)).toHaveAttribute("data-pending", "true");
    await page.getByRole("button", { name: "Next page", exact: true }).click();
    await expect(checkbox(page, 50)).toBeChecked();
    await checkbox(page, 50).click();
    await expect(status(page)).toContainText("124 selected");
    await page
      .getByRole("button", { name: "Preferences", exact: true })
      .click();
    await page
      .getByRole("combobox", { name: "Theme", exact: true })
      .selectOption("dark");
    await page.getByRole("button", { name: "Mail", exact: true }).click();
    await expect(checkbox(page, 50)).not.toBeChecked();
    await page
      .getByRole("combobox", { name: "Filter", exact: true })
      .selectOption("Flagged");
    await expect(
      page.getByRole("button", { name: "Done", exact: true }),
    ).toHaveCount(0);
    await page
      .getByRole("combobox", { name: "Filter", exact: true })
      .selectOption("All");
    await page.getByRole("button", { name: "Select", exact: true }).click();
    await page
      .getByRole("button", { name: "Select all messages", exact: true })
      .click();
    release();
    await count(page, 125);
    await expect(checkbox(page, 0)).toBeChecked();
    expect(await unread(page)).toBe(125);
  } finally {
    release();
  }
});
