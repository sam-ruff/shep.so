import { expect, test, type Page } from "@playwright/test";
import { seed, profile } from "./mailbox-fixture";

async function openConnection(page: Page) {
  await page.route("**/api/capabilities", route => route.fulfill({ json: { mail: true, endpoints: [] } }));
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page.getByRole("button", { name: "Reconnect work@example.test", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Reconnect work@example.test", exact: true });
  await dialog.getByLabel("Incoming password", { exact: true }).fill("fixture-secret-password");
  return dialog;
}

test("connection admission closes before held probes and dismissal fences late credential activation", async ({ page }) => {
  await seed(page);
  let release!: () => void, probes = 0;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route("**/api/mail/probe", async route => { probes++; await held; await route.fulfill({ json: { connected: true } }); });
  const dialog = await openConnection(page);
  await dialog.getByRole("button", { name: "Verify and save account", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect.poll(() => probes).toBe(1);
  await expect(page.getByRole("heading", { name: "Connection: work@example.test", exact: true })).toBeVisible();
  const saved = await page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const store = await BrowserStore.open(profile);
    const attempts = await store.all("accountConnections"); store.close(); return JSON.stringify(attempts);
  }, profile);
  expect(saved).not.toContain("fixture-secret-password");
  await page.getByRole("button", { name: "Dismiss connection for work@example.test", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Connection: work@example.test", exact: true })).toHaveCount(0);
  release();
  await expect.poll(() => probes).toBe(2);
  await expect(page.locator(".account-connection").filter({ hasText: "work@example.test" })).toContainText("Reconnect to refresh or send");
});

test("failed reconnect stays actionable after reload", async ({ page }) => {
  await seed(page);
  await page.route("**/api/mail/probe", route => route.fulfill({ status: 502, json: { error: "Fixture connection refused" } }));
  const dialog = await openConnection(page);
  await dialog.getByRole("button", { name: "Verify and save account", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByText("Fixture connection refused", { exact: true })).toBeVisible();
  await page.reload();
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await expect(page.getByText("Fixture connection refused", { exact: true })).toBeVisible();
  await page.getByRole("heading", { name: "Connection: work@example.test", exact: true }).scrollIntoViewIfNeeded();
  await page.screenshot({ path: "../artifacts/web/connection-failed-reloaded.png" });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(async () => {
    const heading = page.getByRole("heading", { name: "Connection: work@example.test", exact: true });
    await heading.scrollIntoViewIfNeeded();
    await expect(heading).toBeInViewport();
  }).toPass();
  await page.screenshot({ path: "../artifacts/web/connection-failed-mobile-dark.png" });
  await page.getByRole("button", { name: "Retry connection for work@example.test", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Reconnect work@example.test", exact: true }).getByLabel("Incoming password", { exact: true })).toHaveValue("");
});

test("preferences survive legacy mirror failure with their exact field revision", async ({ page }) => {
  await seed(page);
  await page.evaluate(() => {
    const set = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key: string, value: string) {
      if (key.startsWith("shep.preferences.v1.")) throw Error("Fixture legacy mirror full");
      return set.call(this, key, value);
    };
  });
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page.getByRole("combobox", { name: "Theme", exact: true }).selectOption("dark");
  await expect(page.getByRole("region", { name: "Preference saving", exact: true })).toContainText("Preferences saved on this browser");
  await page.reload();
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await expect(page.getByRole("combobox", { name: "Theme", exact: true })).toHaveValue("dark");
  const saved = await page.evaluate(profile => JSON.parse(localStorage.getItem(`shep.profile-preferences.v1.${profile}`)!), profile);
  expect(saved.preferences.appearance).toBe("dark");
  expect(saved.revisions.appearance).toBe(1);
});
