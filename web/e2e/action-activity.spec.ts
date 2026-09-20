import { test, expect, type Page } from "@playwright/test";
import { seed, subject, profile } from "./mailbox-fixture";

const completedCount = (page: Page) => page.evaluate(async profile => {
  const path = "/src/storage.ts", { BrowserStore } = await import(path);
  const store = await BrowserStore.open(profile);
  const completed = await store.intents.activity.page(undefined, true);
  store.close();
  return completed.rows.length;
}, profile);

test("saved individual failures survive unrelated success and reload, with reviewed dismissal", async ({ page }) => {
  await seed(page);
  await page.evaluate(() => {
    const put = IDBObjectStore.prototype.put;
    let fail = true;
    IDBObjectStore.prototype.put = function (...args: Parameters<IDBObjectStore["put"]>) {
      if (fail && this.name === "mail") { fail = false; throw Error("Fixture cache quota failure"); }
      return put.apply(this, args);
    };
  });
  await page.getByRole("button", { name: `Flag ${subject(0)}`, exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Could not confirm");
  await page.getByRole("button", { name: `Flag ${subject(1)}`, exact: true }).click();
  await expect(page.getByRole("button", { name: `Unflag ${subject(1)}`, exact: true })).toBeVisible();
  await expect.poll(() => completedCount(page)).toBe(1);
  await expect(page.getByRole("button", { name: "Activity", exact: true })).toHaveText("Activity (1)");
  await page.reload();
  await page.getByRole("button", { name: "Activity", exact: true }).click();
  const activity = page.getByRole("dialog", { name: "Activity", exact: true });
  await expect(activity).toContainText("Not applied");
  await expect(activity).toContainText("Could not save this message change");
  await expect(activity).not.toContainText("draft open");
  await expect(activity).toContainText("Original folder: INBOX");
  await page.screenshot({ path: "../artifacts/web/action-activity-reloaded.png" });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.screenshot({ path: "../artifacts/web/action-activity-mobile-dark.png" });
  await activity.getByRole("button", { name: "Dismiss reviewed failure" }).click();
  await expect(activity).toContainText("No individual mail changes need attention");
  await activity.getByRole("button", { name: "Close", exact: true }).click();
  await page.reload();
  await expect(page.getByRole("button", { name: "Activity", exact: true })).toHaveText("Activity");
});

test("local admission failure restores the prior optimistic flag while its provider remains held", async ({ page }) => {
  await seed(page);
  await page.evaluate(async () => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const commit = BrowserStore.prototype.commit;
    let hold = true;
    const waiting = new Promise<void>(resolve => Object.assign(window, { releaseAction: resolve }));
    BrowserStore.prototype.commit = async function (changes: any[], lease: any) {
      if (hold && changes.some(c => c.store === "mail")) {
        hold = false;
        Object.assign(window, { actionHeld: true });
        await waiting;
      }
      return commit.call(this, changes, lease);
    };
    const put = IDBObjectStore.prototype.put;
    let admissions = 0;
    IDBObjectStore.prototype.put = function (...args: Parameters<IDBObjectStore["put"]>) {
      if (this.name === "mailActions" && (args[0] as any).status === "Queued" && ++admissions === 2)
        throw Error("Fixture admission full");
      return put.apply(this, args);
    };
  });
  await page.getByRole("button", { name: `Flag ${subject(0)}`, exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).actionHeld)).toBe(true);
  await page.getByRole("button", { name: `Unflag ${subject(0)}`, exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Could not save this change locally");
  await expect(page.getByRole("button", { name: `Unflag ${subject(0)}`, exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as any).actionHeld)).toBe(true);
  await page.screenshot({ path: "../artifacts/web/action-admission-failed-while-held.png" });
  await page.evaluate(() => { (window as any).releaseAction(); (window as any).actionHeld = false; });
  await expect(page.getByRole("button", { name: "Activity", exact: true })).toHaveText("Activity");
  await expect(page.getByRole("button", { name: `Unflag ${subject(0)}`, exact: true })).toBeVisible();
});

test("acknowledged action repairs its saved receipt after reload without repeating the action", async ({ page }) => {
  await seed(page);
  await page.evaluate(async () => {
    const path = "/src/mail_intents.ts", { BrowserIntents } = await import(path);
    const finish = BrowserIntents.prototype.finish;
    let fail = true;
    BrowserIntents.prototype.finish = function (lease: any, status: any) {
      if (fail && status === "applied") { fail = false; return Promise.reject(Error("Fixture lost final receipt")); }
      return finish.call(this, lease, status);
    };
  });
  await page.getByRole("button", { name: `Flag ${subject(0)}`, exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("action record could not finish");
  await expect(page.getByRole("button", { name: `Unflag ${subject(0)}`, exact: true })).toBeVisible();
  await page.reload();
  await page.evaluate(async () => {
    const path = "/src/provider.ts", { GatewayRepository } = await import(path);
    GatewayRepository.prototype.mutateWithReceipt = async () => { throw Error("Repair must not dispatch an action"); };
  });
  await page.getByRole("button", { name: "Activity", exact: true }).click();
  const activity = page.getByRole("dialog", { name: "Activity", exact: true });
  await expect(activity).toContainText("Change acknowledged");
  await page.screenshot({ path: "../artifacts/web/action-activity-repair.png" });
  await activity.getByRole("button", { name: "Repair local cache" }).click();
  await expect(activity).toContainText("No individual mail changes need attention");
  await activity.getByRole("button", { name: "Close", exact: true }).click();
  await expect(page.getByRole("button", { name: `Unflag ${subject(0)}`, exact: true })).toBeVisible();
});

test("queued changes wait for their live tab and resume automatically after it closes", async ({ page, context }) => {
  await seed(page);
  await page.evaluate(async () => {
    const path = "/src/provider.ts", { GatewayRepository } = await import(path);
    GatewayRepository.prototype.mutateWithReceipt = () => new Promise(() => {});
  });
  await page.getByRole("button", { name: `Flag ${subject(0)}`, exact: true }).click();
  await expect(page.getByRole("button", { name: "Activity", exact: true })).toHaveText("Activity (1)");
  const replacement = await context.newPage();
  await replacement.route("**/api/session", route => route.fulfill({ json: { email: "owner@example.test", user_id: profile, csrf: "X".repeat(43) } }));
  await replacement.goto("/");
  await expect(replacement.getByRole("button", { name: `Unflag ${subject(0)}`, exact: true })).toBeVisible();
  await replacement.getByRole("button", { name: "Activity", exact: true }).click();
  await expect(replacement.getByRole("dialog", { name: "Activity", exact: true })).toContainText("Saved, not yet confirmed");
  await page.close();
  await replacement.reload();
  await expect.poll(() => completedCount(replacement)).toBe(1);
  await expect(replacement.getByRole("button", { name: `Unflag ${subject(0)}`, exact: true })).toBeVisible();
  await expect(replacement.getByRole("button", { name: "Activity", exact: true })).toHaveText("Activity");
  await replacement.screenshot({ path: "../artifacts/web/action-queued-resumed.png" });
  await replacement.close();
});

test("an offline change keeps its projected flag after reload and can be cancelled without dispatch", async ({ page }) => {
  await seed(page);
  let dispatched = 0;
  await page.route("**/api/mail/mutate", route => { dispatched++; return route.abort(); });
  await page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const store = await BrowserStore.open(profile);
    const account = await store.get("accounts", "work");
    await store.commit([{ store: "accounts", key: "work", value: { ...account, protocol: "Imap" } }]);
    store.close();
  }, profile);
  await page.reload();
  await page.getByRole("button", { name: `Flag ${subject(0)}`, exact: true }).click();
  await expect(page.getByRole("button", { name: `Unflag ${subject(0)}`, exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Activity", exact: true }).click();
  const activity = page.getByRole("dialog", { name: "Activity", exact: true });
  await expect(activity).toContainText("Waiting for reconnect");
  await page.reload();
  await expect(page.getByRole("button", { name: `Unflag ${subject(0)}`, exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Activity", exact: true }).click();
  await expect(activity).toContainText("Waiting for reconnect");
  await page.screenshot({ path: "../artifacts/web/action-offline-waiting.png" });
  await activity.getByRole("button", { name: "Cancel saved change" }).click();
  await expect(activity).toContainText("No individual mail changes need attention");
  await activity.getByRole("button", { name: "Close", exact: true }).click();
  await expect(page.getByRole("button", { name: `Flag ${subject(0)}`, exact: true })).toBeVisible();
  expect(dispatched).toBe(0);
});

test("cancelling a queued flag restores it while an earlier read operation is still held", async ({ page }) => {
  await seed(page);
  await page.evaluate(async () => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const commit = BrowserStore.prototype.commit;
    let hold = true;
    const pending = new Promise<void>(resolve => Object.assign(window, { releasePrior: resolve }));
    BrowserStore.prototype.commit = async function (changes: any[], lease: any) {
      if (hold && changes.some(c => c.store === "mail")) { hold = false; Object.assign(window, { priorHeld: true }); await pending; }
      return commit.call(this, changes, lease);
    };
  });
  await page.getByRole("button", { name: subject(0), exact: true }).click();
  await page.getByRole("button", { name: "Mark read", exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).priorHeld)).toBe(true);
  await page.getByRole("button", { name: `Flag ${subject(0)}`, exact: true }).click();
  await expect(page.getByRole("button", { name: `Unflag ${subject(0)}`, exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Activity", exact: true }).click();
  const activity = page.getByRole("dialog", { name: "Activity", exact: true });
  await activity.getByRole("button", { name: "Cancel saved change" }).click();
  await expect(activity.getByRole("button", { name: "Cancel saved change" })).toHaveCount(0);
  await activity.getByRole("button", { name: "Close", exact: true }).click();
  await expect(page.getByRole("button", { name: `Flag ${subject(0)}`, exact: true })).toBeVisible();
  await page.evaluate(() => (window as any).releasePrior());
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("confirmed change has durable Undo after reload and retains the inverse result", async ({ page }) => {
  await seed(page);
  await page.getByRole("button", { name: `Flag ${subject(0)}`, exact: true }).click();
  await expect.poll(() => page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const store = await BrowserStore.open(profile);
    const completed = await store.intents.activity.page(undefined, true);
    store.close();
    return completed.rows.length;
  }, profile)).toBe(1);
  await page.reload();
  await expect(page.getByRole("button", { name: `Unflag ${subject(0)}`, exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Activity", exact: true }).click();
  const activity = page.getByRole("dialog", { name: "Activity", exact: true });
  await activity.getByRole("button", { name: "Recent changes" }).click();
  await expect(activity).toContainText("Change confirmed");
  await page.screenshot({ path: "../artifacts/web/action-durable-undo.png" });
  await page.evaluate(async () => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const commit = BrowserStore.prototype.commit;
    let hold = true;
    const pending = new Promise<void>(resolve => Object.assign(window, { releaseUndo: resolve }));
    BrowserStore.prototype.commit = async function (changes: any[], lease: any) {
      if (hold && changes.some(c => c.store === "mail")) { hold = false; Object.assign(window, { undoHeld: true }); await pending; }
      return commit.call(this, changes, lease);
    };
  });
  await activity.getByRole("button", { name: "Undo saved change", exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).undoHeld)).toBe(true);
  await activity.getByRole("button", { name: "Close", exact: true }).click();
  await expect(page.getByRole("button", { name: `Flag ${subject(0)}`, exact: true })).toBeVisible();
  await page.evaluate(() => (window as any).releaseUndo());
  await expect.poll(() => page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const store = await BrowserStore.open(profile);
    const completed = await store.intents.activity.page(undefined, true);
    store.close(); return completed.rows.length;
  }, profile)).toBe(2);
  await page.reload();
  await expect(page.getByRole("button", { name: `Flag ${subject(0)}`, exact: true })).toBeVisible();
});

test("an interrupted dispatch is not repeated and needs a checked review after its live owner closes", async ({ page, context }) => {
  await seed(page);
  await page.evaluate(async () => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const commit = BrowserStore.prototype.commit;
    BrowserStore.prototype.commit = function (changes: any[], lease: any) {
      if (changes.some(c => c.store === "mail")) { Object.assign(window, { dispatchHeld: true }); return new Promise(() => {}); }
      return commit.call(this, changes, lease);
    };
  });
  await page.getByRole("button", { name: `Flag ${subject(0)}`, exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).dispatchHeld)).toBe(true);
  const replacement = await context.newPage();
  await replacement.route("**/api/session", route => route.fulfill({ json: { email: "owner@example.test", user_id: profile, csrf: "X".repeat(43) } }));
  await replacement.goto("/");
  await replacement.getByRole("button", { name: "Activity", exact: true }).click();
  const activity = replacement.getByRole("dialog", { name: "Activity", exact: true });
  await expect(activity.getByRole("button", { name: "Accept current state" })).toBeDisabled();
  await activity.getByRole("checkbox", { name: "I checked the source and destination folders" }).check();
  await activity.getByRole("button", { name: "Accept current state" }).click();
  await expect(activity.getByRole("status")).toContainText("operation in progress");
  await page.close();
  await replacement.reload();
  await expect(replacement.getByRole("button", { name: `Unflag ${subject(0)}`, exact: true })).toBeVisible();
  await replacement.getByRole("button", { name: "Activity", exact: true }).click();
  await expect(activity).toContainText("Started, awaiting confirmation");
  await activity.getByRole("checkbox", { name: "I checked the source and destination folders" }).check();
  await replacement.screenshot({ path: "../artifacts/web/action-interrupted-review.png" });
  await activity.getByRole("button", { name: "Accept current state" }).click();
  await expect(activity).toContainText("No individual mail changes need attention");
  await activity.getByRole("button", { name: "Close", exact: true }).click();
  await expect(replacement.getByRole("button", { name: `Flag ${subject(0)}`, exact: true })).toBeVisible();
  await replacement.close();
});
