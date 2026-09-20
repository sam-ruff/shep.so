import { expect, test, type Page } from "@playwright/test";
import { seed, subject, profile } from "./mailbox-fixture";

async function savedAttempt(page: Page, id: string, state: "checking" | "failed") {
  await page.evaluate(async ({ profile, id, state }) => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const store = await BrowserStore.open(profile);
    const account = await store.get("accounts", "work");
    await store.commit([{ store: "accountConnections", key: "work",
      value: { id, account, state, error: state === "failed" ? "Saved probe refusal" : undefined } }]);
    store.close();
  }, { profile, id, state });
}

test("connection Activity survives reload and dismissal preserves independent mail failures", async ({ page }) => {
  await seed(page);
  await page.evaluate(() => {
    const put = IDBObjectStore.prototype.put;
    let fail = true;
    IDBObjectStore.prototype.put = function (...args: Parameters<IDBObjectStore["put"]>) {
      if (fail && this.name === "mail") { fail = false; throw Error("Fixture quota failure"); }
      return put.apply(this, args);
    };
  });
  await page.getByRole("button", { name: `Flag ${subject(0)}`, exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Could not confirm");
  await savedAttempt(page, "failed-attempt", "failed");
  await page.reload();
  await expect(page.getByRole("button", { name: "Activity", exact: true })).toHaveText("Activity (2)");
  await page.getByRole("button", { name: "Activity", exact: true }).click();
  const activity = page.getByRole("dialog", { name: "Activity", exact: true });
  await expect(activity).toContainText("Saved probe refusal");
  await expect(activity).toContainText("Not applied");
  await expect(activity).toContainText(subject(0));
  await page.screenshot({ path: "../artifacts/web/connection-activity-desktop.png" });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.screenshot({ path: "../artifacts/web/connection-activity-mobile-dark-pending.png" });
  await activity.getByRole("button", { name: "Dismiss connection for work@example.test" }).click();
  await expect(activity).not.toContainText("Saved probe refusal");
  await expect(activity).toContainText("Not applied");
  await page.screenshot({ path: "../artifacts/web/connection-activity-mobile-dark.png" });
  await activity.getByRole("button", { name: "Close", exact: true }).click();
  await page.reload();
  await expect(page.getByRole("button", { name: "Activity", exact: true })).toHaveText("Activity (1)");
});

test("stale Activity dismissal cannot retire a newer connection decision", async ({ page }) => {
  await seed(page);
  await savedAttempt(page, "old-attempt", "failed");
  await page.reload();
  await page.getByRole("button", { name: "Activity", exact: true }).click();
  const activity = page.getByRole("dialog", { name: "Activity", exact: true });
  await expect(activity).toContainText("Saved probe refusal");
  await savedAttempt(page, "new-attempt", "checking");
  await activity.getByRole("button", { name: "Dismiss connection for work@example.test" }).click();
  await expect(activity.getByRole("status")).toContainText("connection attempt changed");
  await activity.getByRole("button", { name: "Refresh activity" }).click();
  await expect(activity).toContainText("Connection not yet confirmed");
  await expect(activity).not.toContainText("Saved probe refusal");
  await activity.getByRole("button", { name: "Reconnect work@example.test" }).click();
  await expect(activity).not.toBeVisible();
  await expect(page.getByRole("heading", { name: "Mail accounts", exact: true })).toBeVisible();
  await page.reload();
  await expect(page.getByRole("button", { name: "Activity", exact: true })).toHaveText("Activity (1)");
});

test("connection progress read failure keeps mail Activity usable and refresh recovers", async ({ page }) => {
  await seed(page);
  await savedAttempt(page, "saved-attempt", "failed");
  await page.reload();
  await expect(page.getByRole("button", { name: "Activity", exact: true })).toHaveText("Activity (1)");
  const restore = await page.evaluateHandle(() => {
    const original = IDBObjectStore.prototype.getAll;
    IDBObjectStore.prototype.getAll = function (...args: Parameters<IDBObjectStore["getAll"]>) {
      if (this.name === "accountConnections") {
        throw new DOMException("Fixture progress read failure", "AbortError");
      }
      return original.apply(this, args);
    };
    return () => { IDBObjectStore.prototype.getAll = original; };
  });
  await page.getByRole("button", { name: "Activity", exact: true }).click();
  const activity = page.getByRole("dialog", { name: "Activity", exact: true });
  await expect(activity).toContainText("Saved connection progress could not load");
  await expect(activity).toContainText("No individual mail changes need attention");
  await expect(activity.getByRole("button", { name: "Outbox", exact: true })).toBeEnabled();
  await restore.evaluate(reset => reset());
  await restore.dispose();
  await activity.getByRole("button", { name: "Refresh activity" }).click();
  await expect(activity).toContainText("Saved probe refusal");
  await expect(activity).not.toContainText("Saved connection progress could not load");
});
