import { expect, test, type Page } from "@playwright/test";
import { seed, profile } from "./mailbox-fixture";

const target = { name: "Projects", delimiter: "/", encoding: "ImapUtf7", selectable: true, no_inferiors: false, non_existent: false, role: null };
async function imap(page: Page) {
  await seed(page);
  await page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const store = await BrowserStore.open(profile);
    for (const id of ["work", "personal"]) {
      const account = await store.get("accounts", id);
      await store.commit([{ store: "accounts", key: id, value: { ...account, protocol: "Imap", port: 993 } }]);
    }
    store.close();
  }, profile);
  await page.reload();
}
async function saved(page: Page) {
  return page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const store = await BrowserStore.open(profile), jobs = await store.folderActions.page();
    const completed = await store.folderActions.page(undefined, true); store.close();
    return [...jobs.rows, ...completed.rows];
  }, profile);
}
async function create(page: Page, account = "work") {
  await page.getByRole("button", { name: `Create folder for ${account}@example.test`, exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Create folder", exact: true });
  await dialog.getByLabel("Folder name", { exact: true }).fill("Projects");
  await dialog.getByRole("button", { name: "Create folder", exact: true }).click();
  await expect(dialog).toContainText("Folder request saved.");
  return dialog;
}
async function reconnect(page: Page, account = "work") {
  await page.route("**/api/capabilities", route => route.fulfill({ json: { mail: true, endpoints: [] } }));
  await page.route("**/api/mail/probe", route => route.fulfill({ json: { connected: true } }));
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page.getByRole("button", { name: `Reconnect ${account}@example.test`, exact: true }).click();
  const dialog = page.getByRole("dialog", { name: `Reconnect ${account}@example.test`, exact: true });
  await dialog.getByLabel("Incoming password", { exact: true }).fill("fixture-folder-password");
  await dialog.getByRole("button", { name: "Verify and save account", exact: true }).click();
  await expect(dialog).toHaveCount(0);
}

test("folder admission survives offline reload and a held CREATE finishes through its saved receipt", async ({ page }) => {
  await imap(page);
  const dialog = await create(page);
  await expect.poll(async () => (await saved(page))[0]?.status).toBe("Waiting");
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await page.reload();
  expect((await saved(page))[0]?.status).toBe("Waiting");
  let release!: () => void, calls = 0;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route("**/api/mail/folders/plan", route => route.fulfill({ json: target }));
  await page.route("**/api/mail/folders/create", async route => { calls++; await held; await route.fulfill({ json: { state: "acknowledged", target } }); });
  await page.route("**/api/mail/folders/inspect", route => route.fulfill({ json: target }));
  await reconnect(page);
  await expect.poll(() => calls).toBe(1);
  await expect.poll(async () => (await saved(page))[0]?.status).toBe("Running");
  await page.getByRole("button", { name: /^Activity/ }).click();
  await page.getByRole("button", { name: "Folder changes", exact: true }).click();
  const activity = page.getByRole("dialog", { name: "Folder changes", exact: true });
  await expect(activity).toContainText("Creating folder");
  await page.screenshot({ path: "../artifacts/web/folder-creation-running.png" });
  release();
  await expect.poll(async () => (await saved(page))[0]?.status).toBe("Succeeded");
  await activity.getByRole("button", { name: "Recent folder changes", exact: true }).click();
  await expect(activity).toContainText("Complete");
  const job = (await saved(page))[0];
  expect(job.receipt).toEqual(target);
  expect(JSON.stringify(job)).not.toContain("fixture-folder-password");
  expect(calls).toBe(1);
});

for (const rejected of [true, false]) test(`folder planning ${rejected ? "rejection needs review" : "disconnection keeps waiting"} without CREATE`, async ({ page }) => {
  await imap(page);
  let plans = 0, creates = 0;
  await page.route("**/api/mail/folders/plan", route => {
    plans++;
    return rejected ? route.fulfill({ json: { state: "rejected" } }) : route.abort("connectionreset");
  });
  await page.route("**/api/mail/folders/create", route => { creates++; return route.abort(); });
  const dialog = await create(page);
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await reconnect(page);
  await expect.poll(() => plans).toBeGreaterThan(0);
  await expect.poll(async () => (await saved(page))[0]?.status).toBe(rejected ? "Rejected" : "Waiting");
  await page.getByRole("button", { name: /^Activity/ }).click();
  await page.getByRole("button", { name: "Folder changes", exact: true }).click();
  const activity = page.getByRole("dialog", { name: "Folder changes", exact: true });
  await expect(activity).toContainText(rejected ? "Needs review" : "Waiting for the account");
  expect(creates).toBe(0);
  const [job] = await saved(page);
  expect(job.target).toBeUndefined(); expect(job.receipt).toBeUndefined();
});

test("unknown folder creation requires read-only checking after restart", async ({ page }) => {
  await imap(page);
  let calls = 0, inspections = 0;
  await page.route("**/api/mail/folders/plan", route => route.fulfill({ json: target }));
  await page.route("**/api/mail/folders/create", route => { calls++; return route.abort("connectionreset"); });
  await page.route("**/api/mail/folders/inspect", route => { inspections++; return route.fulfill({ json: target }); });
  const dialog = await create(page);
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await reconnect(page);
  await expect.poll(async () => (await saved(page))[0]?.status).toBe("Uncertain");
  await page.reload();
  expect((await saved(page))[0]?.status).toBe("Uncertain");
  await reconnect(page);
  await page.getByRole("button", { name: /^Activity/ }).click();
  await page.getByRole("button", { name: "Folder changes", exact: true }).click();
  const activity = page.getByRole("dialog", { name: "Folder changes", exact: true });
  await expect(activity).toContainText("Needs checking");
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.screenshot({ path: "../artifacts/web/folder-creation-uncertain-compact-dark.png" });
  await activity.getByRole("button", { name: "Check Projects", exact: true }).click();
  await expect.poll(async () => (await saved(page))[0]?.status).toBe("Succeeded");
  expect(calls).toBe(1); expect(inspections).toBe(1);
});

test("closing during CREATE preserves uncertainty and never repeats the request on reopen", async ({ page }) => {
  await imap(page);
  let calls = 0, release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route("**/api/mail/folders/plan", route => route.fulfill({ json: target }));
  await page.route("**/api/mail/folders/create", async route => {
    calls++; await held;
    await route.fulfill({ json: { state: "acknowledged", target } }).catch(() => {});
  });
  await page.route("**/api/mail/folders/inspect", route => route.fulfill({ json: target }));
  const dialog = await create(page);
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await reconnect(page);
  await expect.poll(async () => (await saved(page))[0]?.status).toBe("Running");
  await page.reload();
  release();
  await expect.poll(async () => (await saved(page))[0]?.status).toBe("Uncertain");
  await reconnect(page);
  await page.getByRole("button", { name: /^Activity/ }).click();
  await page.getByRole("button", { name: "Folder changes", exact: true }).click();
  await page.getByRole("button", { name: "Check Projects", exact: true }).click();
  await expect.poll(async () => (await saved(page))[0]?.status).toBe("Succeeded");
  expect(calls).toBe(1);
});

test("held mail backlog does not starve a folder on another account", async ({ page }) => {
  await imap(page);
  await reconnect(page); await reconnect(page, "personal");
  let release!: () => void, flags = 0, creates = 0;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route("**/api/mail/flags", async route => { flags++; await held; await route.fulfill({ json: { committed: true } }); });
  await page.route("**/api/mail/folders/plan", route => route.fulfill({ json: target }));
  await page.route("**/api/mail/folders/create", route => { creates++; return route.fulfill({ json: { state: "acknowledged", target } }); });
  await page.route("**/api/mail/folders/inspect", route => route.fulfill({ json: target }));
  await page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const store = await BrowserStore.open(profile);
    await store.intents.register("m000", { starred: true }, "departed-fixture-tab");
    await store.intents.register("m002", { starred: true }, "departed-fixture-tab");
    store.close();
  }, profile);
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  const dialog = await create(page, "personal");
  await expect.poll(() => flags).toBe(1);
  await expect.poll(async () => (await saved(page))[0]?.status).toBe("Succeeded");
  expect(creates).toBe(1); expect(flags).toBe(1);
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  release();
  await expect.poll(() => flags).toBe(2);
});
