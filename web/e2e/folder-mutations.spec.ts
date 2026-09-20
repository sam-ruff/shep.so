import { expect, test, type Page } from "@playwright/test";
import { seed, profile } from "./mailbox-fixture";

const mailbox = (name: string) => ({ name, delimiter: "/", encoding: "Utf8", selectable: true, no_inferiors: false, non_existent: false, role: null });
const catalog = ["INBOX", "Projects", "Projects/Child", "Other"].map(mailbox);
async function setup(page: Page) {
  await seed(page);
  await page.evaluate(async ({ profile, catalog }) => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path), store = await BrowserStore.open(profile);
    const account = await store.get("accounts", "work");
    const changes: any[] = [{ store: "accounts", key: "work", value: { ...account, protocol: "Imap", port: 993 } }, { store: "folderCatalogs", key: "work", value: { account: "work", mailboxes: catalog, complete: true } }];
    for (const id of ["m000", "m002", "m004"]) {
      const mail = await store.get("mail", id);
      changes.push({ store: "mail", key: id, value: { ...mail, core: { ...mail.core, folder: id === "m004" ? "Projects/Child" : "Projects" } } });
    }
    await store.commit(changes); store.close();
  }, { profile, catalog });
  await page.route("**/api/mail/folders/review", route => {
    const { action } = route.request().postDataJSON();
    const destination = action === "Delete" ? null : action.Rename?.name ?? `${action.Move.parent}/Projects`;
    return route.fulfill({ json: { catalog, plan: { source: "Projects", action, parent: action.Move?.parent ? mailbox(action.Move.parent) : null, members: ["Projects/Child", "Projects"].map(name => ({ path: name, mailbox: mailbox(name), listed: true, depth: name === "Projects" ? 0 : 1, destination: destination && destination + name.slice("Projects".length) })) } } });
  });
  await page.reload();
  await reconnect(page);
}
async function reconnect(page: Page) {
  await page.route("**/api/capabilities", route => route.fulfill({ json: { mail: true, endpoints: [] } }));
  await page.route("**/api/mail/probe", route => route.fulfill({ json: { connected: true } }));
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page.getByRole("button", { name: "Reconnect work@example.test", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Reconnect work@example.test", exact: true });
  await dialog.getByLabel("Incoming password", { exact: true }).fill("fixture-folder-password");
  await dialog.getByRole("button", { name: "Verify and save account", exact: true }).click();
  await expect(dialog).toHaveCount(0);
}
async function saved(page: Page) {
  return page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path), store = await BrowserStore.open(profile);
    const pending = await store.folderActions.page(), recent = await store.folderActions.page(undefined, true);
    const mail = await Promise.all(["m000", "m002", "m004"].map(id => store.get("mail", id)));
    const frozen = await store.all("folderMembers"); store.close();
    return { job: [...pending.rows, ...recent.rows][0], mail, frozen };
  }, profile);
}
async function review(page: Page, operation: "Rename" | "Move" | "Delete") {
  await page.getByRole("button", { name: "Manage folders for work@example.test", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Change folder", exact: true });
  await dialog.getByLabel("Folder to change", { exact: true }).selectOption("Projects");
  await dialog.getByLabel("Folder change", { exact: true }).selectOption(operation);
  if (operation === "Rename") await dialog.getByLabel("New folder name", { exact: true }).fill("Renamed");
  if (operation === "Move") await dialog.getByLabel("Destination parent", { exact: true }).selectOption("Other");
  await dialog.getByRole("button", { name: "Review folder change", exact: true }).click();
  await expect(dialog).toContainText("2 folders and 3 cached messages are included.");
  if (operation === "Delete") await expect(dialog).toContainText("This cannot be undone.");
  await dialog.getByRole("button", { name: "Confirm folder change", exact: true }).click();
  await expect(dialog).toContainText("Folder change saved. It will continue in the background.");
  return dialog;
}
async function activity(page: Page) {
  await page.getByRole("button", { name: /^Activity/ }).click();
  await page.getByRole("button", { name: "Folder changes", exact: true }).click();
  return page.getByRole("dialog", { name: "Folder changes", exact: true });
}

test("held rename admits immediately, retains review controls and saves its exact subtree receipt", async ({ page }) => {
  await setup(page);
  let release!: () => void, calls = 0;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route("**/api/mail/folders/step", async route => {
    calls++; const request = route.request().postDataJSON();
    expect(request.completed).toBe(0); expect(request.plan.source).toBe("Projects");
    await held; await route.fulfill({ json: { state: "acknowledged", step: { Rename: { source: "Projects", destination: "Renamed" } } } });
  });
  const dialog = await review(page, "Rename");
  await expect.poll(() => calls).toBe(1);
  expect((await saved(page)).mail.map(mail => mail?.core.folder)).toEqual(["Projects", "Projects", "Projects/Child"]);
  await dialog.getByRole("button", { name: "Review folder changes", exact: true }).click();
  const changes = page.getByRole("dialog", { name: "Folder changes", exact: true });
  await expect(changes).toContainText("Changing folder");
  await page.screenshot({ path: "../artifacts/web/folder-mutation-held-light.png" });
  const refresh = changes.getByRole("button", { name: "Refresh folder changes", exact: true });
  await refresh.focus();
  release();
  await expect.poll(async () => (await saved(page)).job.status).toBe("Succeeded");
  await expect(refresh).toBeFocused();
  expect(calls).toBe(1);
  const result = await saved(page);
  expect(result.mail.map(mail => mail?.core.folder)).toEqual(["Renamed", "Renamed", "Renamed/Child"]);
  expect(result.frozen).toEqual([]);
  expect(result.job.mutation.receipts[0].origin).toBe("acknowledged");
  await changes.getByRole("button", { name: "Recent folder changes", exact: true }).click();
  await expect(changes).toContainText("Complete");
  await changes.getByRole("button", { name: "Close", exact: true }).click();
  await expect(page.locator(".accounts").getByRole("button", { name: "Projects", exact: true })).toHaveCount(0);
  await expect(page.locator(".accounts").getByRole("button", { name: "Renamed", exact: true })).toBeVisible();
});

test("acknowledged move survives cache failure and reload without repeating the server command", async ({ page }) => {
  await setup(page);
  let calls = 0;
  await page.route("**/api/mail/folders/step", route => { calls++; return route.fulfill({ json: { state: "acknowledged", step: { Rename: { source: "Projects", destination: "Other/Projects" } } } }); });
  await page.evaluate(() => {
    const put = IDBObjectStore.prototype.put;
    IDBObjectStore.prototype.put = function (value, key) {
      if (this.name === "mail" && value.core?.folder.startsWith("Other/Projects")) throw Error("Fixture cache save refused");
      return put.call(this, value, key);
    };
  });
  const dialog = await review(page, "Move");
  await expect.poll(async () => (await saved(page)).job.status).toBe("Repair");
  expect((await saved(page)).job.mutation.receipt.origin).toBe("acknowledged");
  await dialog.getByRole("button", { name: "Review folder changes", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Folder changes", exact: true })).toContainText("Saving on this device");
  await page.reload();
  await expect.poll(async () => (await saved(page)).job.status).toBe("Succeeded");
  expect(calls).toBe(1);
  expect((await saved(page)).mail.map(mail => mail?.core.folder)).toEqual(["Other/Projects", "Other/Projects", "Other/Projects/Child"]);
});

test("unknown deletion uses a checked explicit cache decision and never repeats that step", async ({ page }) => {
  await setup(page);
  let calls = 0, checks = 0;
  await page.route("**/api/mail/folders/step", route => {
    calls++; const request = route.request().postDataJSON();
    if (request.completed === 0) return route.fulfill({ json: { state: "uncertain" } });
    return route.fulfill({ json: { state: "acknowledged", step: { Delete: { source: "Projects" } } } });
  });
  await page.route("**/api/mail/folders/check-step", route => { checks++; return route.fulfill({ json: { state: "applied", catalog: catalog.filter(mailbox => mailbox.name !== "Projects/Child") } }); });
  const dialog = await review(page, "Delete");
  await expect.poll(async () => (await saved(page)).job.status).toBe("Uncertain");
  expect((await saved(page)).mail.filter(Boolean)).toHaveLength(3);
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await page.reload();
  expect(calls).toBe(1);
  await reconnect(page);
  const changes = await activity(page);
  await expect(changes).toContainText("Needs checking");
  await changes.getByRole("button", { name: "Check Projects", exact: true }).click();
  await expect.poll(() => checks).toBe(1);
  await expect.poll(async () => (await saved(page)).job.mutation.checked).toBe("applied");
  await changes.getByRole("button", { name: "Refresh folder changes", exact: true }).click();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => { document.documentElement.dataset.theme = "dark"; });
  await page.screenshot({ path: "../artifacts/web/folder-mutation-check-dark-compact.png" });
  await changes.getByRole("button", { name: "Accept checked deletion of Projects", exact: true }).click();
  const confirm = page.getByRole("dialog", { name: "Accept checked folder deletion", exact: true });
  await confirm.getByRole("button", { name: "Accept checked deletion", exact: true }).click();
  await expect.poll(async () => (await saved(page)).job.status).toBe("Succeeded");
  expect(calls).toBe(2); expect(checks).toBe(1);
  const result = await saved(page);
  expect(result.mail.filter(Boolean)).toEqual([]);
  expect(result.job.mutation.receipts.map((receipt: { origin: string }) => receipt.origin)).toEqual(["observed", "acknowledged"]);
});

test("a replaced cache can stop tracking an acknowledged change only after a current checked review", async ({ page }) => {
  await setup(page);
  let calls = 0, checks = 0;
  await page.route("**/api/mail/folders/step", route => { calls++; return route.fulfill({ json: { state: "acknowledged", step: { Rename: { source: "Projects", destination: "Renamed" } } } }); });
  await page.route("**/api/mail/folders/check-step", route => { checks++; return route.fulfill({ json: { state: "applied", catalog: [mailbox("Renamed"), mailbox("Renamed/Child")] } }); });
  await page.evaluate(() => {
    const put = IDBObjectStore.prototype.put;
    IDBObjectStore.prototype.put = function (value, key) {
      if (this.name === "mail" && value.core?.folder.startsWith("Renamed")) throw Error("Fixture cache save refused");
      return put.call(this, value, key);
    };
  });
  const dialog = await review(page, "Rename");
  await expect.poll(async () => (await saved(page)).job.status).toBe("Repair");
  await page.evaluate(async profile => {
    const path = "/src/storage.ts", { openMailDatabase } = await import(path), db = await openMailDatabase(profile);
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(["mailMetadata", "cacheState"], "readwrite");
      const mail = tx.objectStore("mailMetadata").get("m000");
      mail.onsuccess = () => tx.objectStore("mailMetadata").put({ ...mail.result, lineage: "fixture-replacement-lineage" }, "m000");
      const state = tx.objectStore("cacheState").get("mail");
      state.onsuccess = () => tx.objectStore("cacheState").put({ ...state.result, epoch: "fixture-replacement-cache", revision: state.result.revision + 1 }, "mail");
      tx.oncomplete = () => resolve(); tx.onabort = () => reject(tx.error);
    }); db.close();
  }, profile);
  await dialog.getByRole("button", { name: "Review folder changes", exact: true }).click();
  const changes = page.getByRole("dialog", { name: "Folder changes", exact: true });
  await expect(changes.getByRole("button", { name: "Keep cached mail and stop tracking Projects", exact: true })).toHaveCount(0);
  await changes.getByRole("button", { name: "Check Projects", exact: true }).click();
  await expect.poll(async () => (await saved(page)).job.mutation.checkedCache?.epoch).toBe("fixture-replacement-cache");
  await changes.getByRole("button", { name: "Refresh folder changes", exact: true }).click();
  await changes.getByRole("button", { name: "Keep cached mail and stop tracking Projects", exact: true }).click();
  let confirm = page.getByRole("dialog", { name: "Keep cached mail and stop tracking", exact: true });
  await expect(confirm).toContainText("The server change remains.");
  await page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path), store = await BrowserStore.open(profile);
    const mail = await store.get("mail", "m001");
    await store.commit([{ store: "mail", key: "m001", value: { ...mail, core: { ...mail.core, starred: true } } }]); store.close();
  }, profile);
  await confirm.getByRole("button", { name: "Keep cached mail and stop tracking", exact: true }).click();
  await expect(changes).toContainText("cache changed after the check");
  expect((await saved(page)).job.status).toBe("Repair");
  await changes.getByRole("button", { name: "Check Projects", exact: true }).click();
  await expect.poll(() => checks).toBe(2);
  await expect.poll(async () => (await saved(page)).job.status).toBe("Repair");
  await changes.getByRole("button", { name: "Refresh folder changes", exact: true }).click();
  await changes.getByRole("button", { name: "Keep cached mail and stop tracking Projects", exact: true }).click();
  confirm = page.getByRole("dialog", { name: "Keep cached mail and stop tracking", exact: true });
  await confirm.getByRole("button", { name: "Keep cached mail and stop tracking", exact: true }).click();
  await expect.poll(async () => (await saved(page)).job.status).toBe("Dismissed");
  const retained = await saved(page);
  expect(retained.mail.filter(Boolean)).toHaveLength(3);
  expect(retained.frozen).toHaveLength(3);
  expect(retained.job.mutation.receipt.origin).toBe("acknowledged");
  await changes.getByRole("button", { name: "Recent folder changes", exact: true }).click();
  await expect(changes).toContainText("No longer tracked");
  await expect(changes).toContainText("cached mail was kept");
  await page.reload();
  expect((await saved(page)).job.status).toBe("Dismissed");
  expect(calls).toBe(1);
});

test("an unknown rename never adopts old message identities from a folder listing", async ({ page }) => {
  await setup(page);
  let calls = 0, checks = 0;
  await page.route("**/api/mail/folders/step", route => { calls++; return route.fulfill({ json: { state: "uncertain" } }); });
  await page.route("**/api/mail/folders/check-step", route => {
    checks++;
    return checks === 1 ? route.abort() : route.fulfill({ json: { state: "applied", catalog: [mailbox("Renamed"), mailbox("Renamed/Child")] } });
  });
  const dialog = await review(page, "Rename");
  await expect.poll(async () => (await saved(page)).job.status).toBe("Uncertain");
  await dialog.getByRole("button", { name: "Review folder changes", exact: true }).click();
  const changes = page.getByRole("dialog", { name: "Folder changes", exact: true });
  await changes.getByRole("button", { name: "Check Projects", exact: true }).click();
  await expect.poll(() => checks).toBe(1);
  await expect.poll(async () => (await saved(page)).job.status).toBe("Uncertain");
  await changes.getByRole("button", { name: "Refresh folder changes", exact: true }).click();
  await changes.getByRole("button", { name: "Check Projects", exact: true }).click();
  await expect.poll(async () => (await saved(page)).job.mutation.checked).toBe("applied");
  await changes.getByRole("button", { name: "Refresh folder changes", exact: true }).click();
  await expect(changes).toContainText("message identities are not proven by this listing");
  await expect(changes.getByRole("button", { name: "Retry Projects", exact: true })).toHaveCount(0);
  await expect(changes.getByRole("button", { name: /^Accept checked deletion/ })).toHaveCount(0);
  expect((await saved(page)).mail.map(mail => mail?.core.folder)).toEqual(["Projects", "Projects", "Projects/Child"]);
  await changes.getByRole("button", { name: "Stop tracking Projects", exact: true }).click();
  const confirm = page.getByRole("dialog", { name: "Stop tracking folder change", exact: true });
  await confirm.getByRole("button", { name: "Stop tracking", exact: true }).click();
  await expect.poll(async () => (await saved(page)).job.status).toBe("Dismissed");
  expect((await saved(page)).mail.filter(Boolean)).toHaveLength(3);
  expect(calls).toBe(1); expect(checks).toBe(2);
});

test("folder controls protect a queued message's older filing destination until it returns to Drafts", async ({ page }) => {
  await setup(page);
  let mutations = 0, sends = 0;
  await page.route("**/api/mail/folders/step", route => { mutations++; return route.abort(); });
  await page.route("**/api/mail/outgoing/reserve", route => { sends++; return route.abort(); });
  await page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path), store = await BrowserStore.open(profile);
    const account = await store.get("accounts", "work");
    const draft = { id: "frozen-filing-draft", accountId: "work", revision: 1, to: "recipient@example.test", cc: "", bcc: "", subject: "Frozen older filing destination", body: "Exact queued text remains editable", attachments: [] };
    const record = { id: "queued:filing-fixture", draft, account: { ...account, sent_copy: "Automatic", sent_folder: "Projects" }, state: "queued" };
    await store.commit([{ store: "drafts", key: draft.id, value: draft }, { store: "outgoing", key: draft.id, value: record }]); store.close();
  }, profile);
  await page.getByRole("button", { name: "Manage folders for work@example.test", exact: true }).click();
  let dialog = page.getByRole("dialog", { name: "Change folder", exact: true });
  await dialog.getByLabel("Folder to change", { exact: true }).selectOption("Projects");
  await dialog.getByLabel("Folder change", { exact: true }).selectOption("Delete");
  await dialog.getByRole("button", { name: "Review folder change", exact: true }).click();
  await expect(dialog).toContainText("An Outbox entry owns a filing destination");
  await expect(dialog.getByRole("button", { name: "Confirm folder change", exact: true })).toBeHidden();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await page.getByRole("button", { name: "Outbox", exact: true }).click();
  const outbox = page.getByRole("dialog", { name: "Outbox", exact: true });
  await expect(outbox).toContainText("Frozen older filing destination");
  await outbox.getByRole("button", { name: "Return to drafts", exact: true }).click();
  await expect(outbox).toContainText("No outgoing messages need attention");
  await outbox.getByRole("button", { name: "Close", exact: true }).click();
  await page.getByRole("button", { name: "Manage folders for work@example.test", exact: true }).click();
  dialog = page.getByRole("dialog", { name: "Change folder", exact: true });
  await dialog.getByLabel("Folder to change", { exact: true }).selectOption("Projects");
  await dialog.getByLabel("Folder change", { exact: true }).selectOption("Delete");
  await dialog.getByRole("button", { name: "Review folder change", exact: true }).click();
  await expect(dialog.getByRole("button", { name: "Confirm folder change", exact: true })).toBeVisible();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await page.getByRole("button", { name: "Drafts", exact: true }).click();
  await page.getByRole("button", { name: "Frozen older filing destination", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "Message", exact: true })).toHaveValue("Exact queued text remains editable");
  expect(mutations).toBe(0); expect(sends).toBe(0);
});
