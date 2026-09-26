import { expect, test } from "@playwright/test";
import { profile, seed } from "./mailbox-fixture";

const source = { id: "https://calendar.example.test/home/", name: "Home", read_only: false };

for (const compact of [false, true]) test(`CalDAV setup is durable before a held check and requires transient credential re-entry after reload (${compact ? "compact dark" : "desktop light"})`, async ({ page }) => {
  if (compact) { await page.setViewportSize({ width: 390, height: 844 }); await page.emulateMedia({ colorScheme: "dark" }); }
  let release!: () => void, checks = 0, mutations = 0;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route("**/api/calendar", async route => {
    const { operation } = route.request().postDataJSON();
    if (operation.kind === "endpoints") return route.fulfill({ json: { state: "observed", value: { endpoints: [{ id: "approved-home", name: "Approved Home" }] } } });
    if (operation.kind === "cal_dav" && operation.operation.kind === "sources") {
      checks++;
      expect(operation).toMatchObject({ endpoint_id: "approved-home", username: "sam", password: checks === 1 ? "first-secret" : "second-secret" });
      if (checks === 1) await held;
      return route.fulfill({ json: { state: "observed", value: { sources: [source] } } });
    }
    if (operation.kind === "cal_dav" && operation.operation.kind === "mutate") {
      mutations++;
      const mutation = operation.operation.mutation;
      return route.fulfill({ json: { state: "acknowledged", value: { receipt: {
        request_id: operation.operation.request_id, before: mutation.save.before,
        after: { ...mutation.save.after, etag: "v2" },
      } } } });
    }
    return route.fulfill({ json: { state: "observed", value: { sources: [], events: [] } } });
  });
  await seed(page);
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page.getByRole("button", { name: "Manage CalDAV connections", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "CalDAV connections", exact: true });
  await dialog.getByRole("button", { name: "Add CalDAV connection", exact: true }).click();
  await dialog.getByLabel("CalDAV username", { exact: true }).fill("sam");
  await dialog.getByLabel("CalDAV password", { exact: true }).fill("first-secret");
  await dialog.getByRole("button", { name: "Save CalDAV connection", exact: true }).click();
  await expect(dialog).toContainText("Connection saved. Checking");
  const prepared = await page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path), store = await BrowserStore.open(profile);
    const rows = await store.calendar.connections(); store.close(); return rows;
  }, profile);
  expect(prepared).toHaveLength(1); expect(prepared[0]).toMatchObject({ endpoint_id: "approved-home", username: "sam", status: "Prepared" });
  expect(JSON.stringify(prepared)).not.toContain("first-secret");
  release();
  await expect(dialog).toContainText("Connection saved. Re-enter its password after reopening Shep.");
  await page.screenshot({ path: `../artifacts/web/caldav-connected-${compact ? "dark-compact" : "light"}.png`, fullPage: true });

  await page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path), store = await BrowserStore.open(profile);
    const start = new Date().toISOString(), before = { id: "event.ics", source_id: "https://calendar.example.test/home/", title: "Original", start,
      end: new Date(Date.parse(start) + 3600000).toISOString(), location: "", description: "", all_day: false, etag: "v1", remote_url: "event.ics" };
    await store.calendar.sync(before.source_id, new Date(Date.parse(start) - 86400000).toISOString(), new Date(Date.parse(start) + 86400000).toISOString(), [before], await store.calendar.observationRevision());
    await store.calendar.admit({ id: crypto.randomUUID(), key: `${before.source_id.length}:${before.source_id}${before.id}`, owner: "closed-tab",
      mutation: { save: { before, after: { ...before, title: "Recovered after restart" } } } });
    store.close();
  }, profile);

  await page.reload();
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page.getByRole("button", { name: "Manage CalDAV connections", exact: true }).click();
  const reopened = page.getByRole("dialog", { name: "CalDAV connections", exact: true });
  await reopened.getByRole("button", { name: "Re-enter password", exact: true }).click();
  await reopened.getByLabel("CalDAV password", { exact: true }).fill("second-secret");
  await reopened.getByRole("button", { name: "Save CalDAV connection", exact: true }).click();
  await expect(reopened).toContainText("Connection saved. Re-enter its password after reopening Shep.");
  expect(checks).toBe(2);
  const durable = await page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path), store = await BrowserStore.open(profile);
    const rows = await store.calendar.connections(); store.close(); return rows;
  }, profile);
  expect(durable[0]).toMatchObject({ status: "Active", revision: 2 });
  expect(JSON.stringify(durable)).not.toContain("second-secret");
  await reopened.getByRole("button", { name: "Close", exact: true }).click();
  await page.getByRole("button", { name: "Calendar", exact: true }).click();
  await page.getByRole("button", { name: "Calendar changes", exact: true }).click();
  const activity = page.getByRole("dialog", { name: "Calendar changes", exact: true });
  await expect(activity).toContainText("Waiting for Calendar access");
  await activity.getByRole("button", { name: "Retry Recovered after restart", exact: true }).click();
  await expect.poll(() => mutations).toBe(1);
  await activity.getByRole("button", { name: "Recent Calendar changes", exact: true }).click();
  await expect(activity).toContainText("Complete");
});

test("a changed CalDAV binding is refused before provider dispatch and remains recoverable in Activity", async ({ page }) => {
  let providerCalls = 0;
  await page.route("**/api/calendar", async route => {
    providerCalls++;
    return route.fulfill({ json: { state: "observed", value: { sources: [], events: [] } } });
  });
  await seed(page);
  await page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path), store = await BrowserStore.open(profile);
    const prepared = await store.calendar.admitConnection({ id: "changed", endpoint_id: "approved-home", username: "sam" });
    const active = await store.calendar.activateConnection(prepared);
    const source = { id: "https://calendar.example.test/changed/", name: "Changed", read_only: false, connection_id: active.id };
    const start = new Date().toISOString(), before = { id: "changed.ics", source_id: source.id, title: "Binding review", start,
      end: new Date(Date.parse(start) + 3600000).toISOString(), location: "", description: "", all_day: false, etag: "v1", remote_url: "changed.ics" };
    await store.calendar.saveSources([source]);
    await store.calendar.sync(source.id, new Date(Date.parse(start) - 86400000).toISOString(), new Date(Date.parse(start) + 86400000).toISOString(), [before], await store.calendar.observationRevision());
    await store.calendar.admit({ id: crypto.randomUUID(), key: `${source.id.length}:${source.id}${before.id}`, owner: "closed-tab",
      mutation: { save: { before, after: { ...before, title: "Must not dispatch" } } } });
    await store.calendar.activateConnection(active);
    store.close();
  }, profile);
  await page.getByRole("button", { name: "Calendar", exact: true }).click();
  await page.getByRole("button", { name: "Calendar changes", exact: true }).click();
  const activity = page.getByRole("dialog", { name: "Calendar changes", exact: true });
  await activity.getByRole("button", { name: "Refresh Calendar changes", exact: true }).click();
  await expect.poll(() => page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path), store = await BrowserStore.open(profile);
    const rows = await store.calendar.page(); store.close(); return rows.rows[0]?.status;
  }, profile)).toBe("Rejected");
  await activity.getByRole("button", { name: "Refresh Calendar changes", exact: true }).click();
  await expect(activity).toContainText("Needs review");
  await expect(activity).toContainText("CalDAV connection changed");
  expect(providerCalls).toBe(0);
  await expect(activity.getByRole("button", { name: "Check Must not dispatch", exact: true })).toBeVisible();
});
