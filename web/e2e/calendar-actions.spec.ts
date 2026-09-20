import { expect, test, type Page } from "@playwright/test";
import { seed, profile } from "./mailbox-fixture";

const source = { id: "owned-calendar", name: "Fixture Calendar", read_only: false };
function event() {
  const now = new Date(), start = new Date(Date.UTC(now.getFullYear(), now.getMonth(), 3, 10)).toISOString();
  return { id: "remote-event", source_id: source.id, title: "Planning", start, end: new Date(Date.parse(start) + 3600000).toISOString(), location: "Room A", description: "Keep the original provider description", all_day: false, etag: "v1", remote_url: "remote-event" };
}
async function jobs(page: Page) {
  return page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path), store = await BrowserStore.open(profile);
    const active = await store.calendar.page();
    const completed = await store.calendar.page(undefined, true); store.close();
    return [...active.rows, ...completed.rows];
  }, profile);
}
async function openCalendar(page: Page) {
  await page.getByRole("button", { name: "Calendar", exact: true }).click();
  await page.getByRole("button", { name: "Refresh Calendar", exact: true }).click();
}

test("real edit and delete controls admit immediately while provider is held and preserve newer exact identity", async ({ page }) => {
  let saved: ReturnType<typeof event> | null = event();
  const mutations: any[] = [];
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  let releaseDelete!: () => void;
  const heldDelete = new Promise<void>(resolve => { releaseDelete = resolve; });
  await page.route("**/api/calendar", async route => {
    const { operation } = route.request().postDataJSON();
    if (operation.kind === "sources") return route.fulfill({ json: { state: "observed", value: { sources: [source] } } });
    if (operation.kind === "events") return route.fulfill({ json: { state: "observed", value: { events: saved ? [saved] : [] } } });
    if (operation.kind === "inspect") return route.fulfill({ json: { state: "observed", value: { current: saved } } });
    mutations.push(operation);
    if (mutations.length === 1) await held;
    const before = operation.mutation.save?.before ?? operation.mutation.delete.before;
    if (operation.mutation.delete) { await heldDelete; saved = null; }
    else saved = { ...operation.mutation.save.after, etag: `v${mutations.length + 1}` };
    await route.fulfill({ json: { state: "acknowledged", value: { receipt: { request_id: operation.request_id, before, after: saved } } } });
  });
  await seed(page); await openCalendar(page);
  await page.getByRole("button", { name: "Planning", exact: true }).click();
  const editor = page.getByRole("dialog", { name: "Edit event", exact: true });
  await editor.getByLabel("Event title", { exact: true }).fill("First saved title");
  await editor.getByRole("button", { name: "Save event", exact: true }).click();
  await expect(editor).toHaveCount(0);
  await expect(page.getByRole("button", { name: "First saved title", exact: true })).toBeVisible();
  await expect.poll(() => mutations.length).toBe(1);
  expect(mutations[0].mutation.save.before).toEqual(event());
  expect(mutations[0].mutation.save.after).toMatchObject({ id: "remote-event", etag: "v1", remote_url: "remote-event", description: event().description, all_day: false });
  await page.screenshot({ path: "../artifacts/web/calendar-pending-light.png", fullPage: true });
  await page.getByRole("button", { name: "First saved title", exact: true }).click();
  await editor.getByLabel("Event title", { exact: true }).fill("Latest title");
  await editor.getByRole("button", { name: "Save event", exact: true }).click();
  await expect(editor).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Latest title", exact: true })).toBeVisible();
  expect(mutations).toHaveLength(1);
  const pendingButton = page.getByRole("button", { name: "Latest title", exact: true });
  const heldTarget = await pendingButton.boundingBox();
  expect(heldTarget).not.toBeNull();
  await page.mouse.move(heldTarget!.x + heldTarget!.width / 2, heldTarget!.y + heldTarget!.height / 2);
  await page.mouse.down();
  release();
  await expect.poll(() => mutations.length).toBe(2);
  expect(mutations[1].mutation.save.before).toMatchObject({ id: "remote-event", title: "First saved title", etag: "v2" });
  expect(mutations[1].mutation.save.after).toMatchObject({ id: "remote-event", title: "Latest title", etag: "v2", remote_url: "remote-event" });
  await expect.poll(async () => (await jobs(page)).filter(job => job.status === "Succeeded").length).toBe(2);
  await page.mouse.up();
  await expect(editor).toBeVisible();
  await editor.getByRole("button", { name: "Delete event", exact: true }).click();
  await page.getByRole("dialog", { name: "Delete event?", exact: true }).getByRole("button", { name: "Delete this event", exact: true }).click();
  await expect(editor).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Latest title", exact: true })).toHaveCount(0);
  await expect.poll(() => mutations.length).toBe(3);
  expect(mutations[2].mutation.delete.before.etag).toBe("v3");
  releaseDelete();
  await expect.poll(async () => (await jobs(page)).filter(job => job.status === "Succeeded").length).toBe(3);
  await page.reload(); await page.getByRole("button", { name: "Calendar", exact: true }).click();
  await expect(page.getByRole("button", { name: "Latest title", exact: true })).toHaveCount(0);
});

test("lost create response survives reload without replay and exact checked adoption stays distinct from success", async ({ page }) => {
  let saved: any = null, mutations = 0, inspections = 0;
  await page.route("**/api/calendar", async route => {
    const { operation } = route.request().postDataJSON();
    if (operation.kind === "sources") return route.fulfill({ json: { state: "observed", value: { sources: [source] } } });
    if (operation.kind === "events") return route.fulfill({ json: { state: "observed", value: { events: saved ? [saved] : [] } } });
    if (operation.kind === "inspect") { inspections++; return route.fulfill({ json: { state: "observed", value: { current: saved } } }); }
    mutations++;
    expect(operation.mutation.save.before).toBeNull();
    saved = { ...operation.mutation.save.after, id: `shep${operation.request_id.replaceAll("-", "")}`, etag: "created-v1", remote_url: "created-event" };
    await route.abort("failed");
  });
  await seed(page); await openCalendar(page);
  await expect(page.getByRole("button", { name: "Refresh Calendar", exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "New event", exact: true }).click();
  const editor = page.getByRole("dialog", { name: "New event", exact: true });
  await editor.getByLabel("Event title", { exact: true }).fill("Unconfirmed appointment");
  await editor.getByRole("button", { name: "Save event", exact: true }).click();
  await expect(editor).toHaveCount(0);
  await expect.poll(async () => (await jobs(page))[0]?.status).toBe("Uncertain");
  await page.reload(); await page.getByRole("button", { name: "Calendar", exact: true }).click();
  await expect(page.getByRole("button", { name: "Unconfirmed appointment", exact: true })).toBeVisible();
  expect(mutations).toBe(1);
  await page.getByRole("button", { name: "Calendar changes", exact: true }).click();
  const history = page.getByRole("dialog", { name: "Calendar changes", exact: true });
  await expect(history).toContainText("Needs checking");
  await history.getByRole("button", { name: "Check Unconfirmed appointment", exact: true }).click();
  await expect(history.getByRole("button", { name: "Review checked state for Unconfirmed appointment", exact: true })).toBeVisible();
  expect(inspections).toBe(1); expect(mutations).toBe(1);
  await page.emulateMedia({ colorScheme: "dark" }); await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: "../artifacts/web/calendar-check-dark-compact.png", fullPage: true });
  await history.getByRole("button", { name: "Review checked state for Unconfirmed appointment", exact: true }).click();
  const review = page.getByRole("dialog", { name: "Use checked Calendar state?", exact: true });
  await review.getByRole("button", { name: "Use checked server state", exact: true }).click();
  await expect.poll(async () => (await jobs(page))[0]?.status).toBe("Dismissed");
  await history.getByRole("button", { name: "Close", exact: true }).click();
  await expect(page.getByRole("button", { name: "Unconfirmed appointment", exact: true })).toHaveCount(1);
  expect(mutations).toBe(1);
});

test("late local admission preserves newer editor text and a failed write requires explicit discard or retry", async ({ page }) => {
  let saved = event();
  const mutations: any[] = [];
  await page.route("**/api/calendar", async route => {
    const { operation } = route.request().postDataJSON();
    if (operation.kind === "sources") return route.fulfill({ json: { state: "observed", value: { sources: [source] } } });
    if (operation.kind === "events") return route.fulfill({ json: { state: "observed", value: { events: [saved] } } });
    mutations.push(operation);
    saved = { ...operation.mutation.save.after, etag: `v${mutations.length + 1}` };
    await route.fulfill({ json: { state: "acknowledged", value: { receipt: { request_id: operation.request_id, before: operation.mutation.save.before, after: saved } } } });
  });
  await seed(page); await openCalendar(page);
  await page.getByRole("button", { name: "Planning", exact: true }).click();
  await page.evaluate(async () => {
    const path = "/src/calendar_actions.ts", { BrowserCalendar } = await import(path);
    const original = BrowserCalendar.prototype.admit;
    let once = true, release!: () => void;
    const held = new Promise<void>(resolve => { release = resolve; });
    (window as any).releaseCalendarAdmission = release;
    BrowserCalendar.prototype.admit = async function(input: unknown) {
      const result = await original.call(this, input);
      if (once) { once = false; await held; throw Error("Calendar admission reply lost after commit"); }
      return result;
    };
  });
  const editor = page.getByRole("dialog", { name: "Edit event", exact: true });
  await editor.getByLabel("Event title", { exact: true }).fill("Earlier saved title");
  await editor.getByRole("button", { name: "Save event", exact: true }).click();
  await expect(editor).toContainText("Saving on this browser");
  await editor.getByLabel("Event title", { exact: true }).fill("Text entered during admission");
  await editor.getByLabel("Description", { exact: true }).fill("Newer description remains editable");
  await editor.getByRole("button", { name: "Close", exact: true }).click();
  await expect(editor).toBeVisible();
  await page.evaluate(() => (window as any).releaseCalendarAdmission());
  await expect(editor).toContainText("Calendar admission reply lost after commit");
  await expect.poll(() => mutations.length).toBe(1);
  await editor.getByRole("button", { name: "Delete event", exact: true }).click();
  await expect(editor).toContainText("confirm the earlier saved request before deleting");
  await editor.getByRole("button", { name: "Save event", exact: true }).click();
  await expect(editor).toContainText("Your newer edits are still here");
  await expect(editor.getByLabel("Event title", { exact: true })).toHaveValue("Text entered during admission");
  await editor.getByRole("button", { name: "Save event", exact: true }).click();
  await expect(editor).toHaveCount(0);
  await expect.poll(() => mutations.length).toBe(2);
  expect(mutations[1].mutation.save.after).toMatchObject({ title: "Text entered during admission", description: "Newer description remains editable", id: "remote-event", remote_url: "remote-event", etag: "v2" });
  await expect.poll(async () => (await jobs(page)).filter(job => job.status === "Succeeded").length).toBe(2);
  await page.getByRole("button", { name: "Text entered during admission", exact: true }).click();
  await page.evaluate(() => {
    const put = IDBObjectStore.prototype.put;
    let fail = true;
    IDBObjectStore.prototype.put = function(...args: Parameters<IDBObjectStore["put"]>) {
      if (this.name === "calendarActions" && fail) { fail = false; throw new DOMException("Calendar local fixture storage unavailable", "QuotaExceededError"); }
      return put.apply(this, args);
    };
  });
  await editor.getByLabel("Event title", { exact: true }).fill("Retained after local failure");
  await editor.getByRole("button", { name: "Save event", exact: true }).click();
  await expect(editor).toContainText("Calendar local fixture storage unavailable");
  expect(mutations).toHaveLength(2);
  await editor.getByRole("button", { name: "Close", exact: true }).click();
  const discard = page.getByRole("dialog", { name: "Discard unsaved event edits?", exact: true });
  await discard.getByRole("button", { name: "Keep editing", exact: true }).click();
  await expect(editor.getByLabel("Event title", { exact: true })).toHaveValue("Retained after local failure");
  await editor.getByRole("button", { name: "Save event", exact: true }).click();
  await expect(editor).toHaveCount(0);
  await expect.poll(() => mutations.length).toBe(3);
  expect(mutations[2].mutation.save.after.description).toBe("Newer description remains editable");
});

for (const rejected of [false, true]) test(`lost create admission retains newer text after ${rejected ? "provider rejection" : "completed creation"}`, async ({ page }) => {
  const mutations: any[] = [];
  await page.route("**/api/calendar", async route => {
    const { operation } = route.request().postDataJSON();
    if (operation.kind === "sources") return route.fulfill({ json: { state: "observed", value: { sources: [source] } } });
    if (operation.kind === "events") return route.fulfill({ json: { state: "observed", value: { events: [] } } });
    mutations.push(operation);
    if (rejected) return route.fulfill({ json: { state: "rejected", error: "Fixture permanent refusal" } });
    const before = operation.mutation.save.before;
    const after = { ...operation.mutation.save.after, id: before?.id ?? `shep${operation.request_id.replaceAll("-", "")}`, etag: `v${mutations.length}`, remote_url: "created-event" };
    return route.fulfill({ json: { state: "acknowledged", value: { receipt: { request_id: operation.request_id, before, after } } } });
  });
  await seed(page); await openCalendar(page);
  await expect(page.getByRole("button", { name: "Refresh Calendar", exact: true })).toBeEnabled();
  await page.evaluate(async () => {
    const path = "/src/calendar_actions.ts", { BrowserCalendar } = await import(path);
    const admit = BrowserCalendar.prototype.admit; let once = true;
    BrowserCalendar.prototype.admit = async function(input: unknown) {
      const result = await admit.call(this, input);
      if (once) { once = false; throw Error("Committed admission reply lost"); }
      return result;
    };
  });
  await page.getByRole("button", { name: "New event", exact: true }).click();
  const editor = page.getByRole("dialog", { name: "New event", exact: true });
  await editor.getByLabel("Event title", { exact: true }).fill("First create");
  await editor.getByRole("button", { name: "Save event", exact: true }).click();
  await expect(editor).toContainText("Committed admission reply lost");
  await expect.poll(async () => (await jobs(page))[0]?.status).toBe(rejected ? "Rejected" : "Succeeded");
  await editor.getByLabel("Event title", { exact: true }).fill("Newer retained title");
  await editor.getByRole("button", { name: "Save event", exact: true }).click();
  await expect(editor.getByLabel("Event title", { exact: true })).toHaveValue("Newer retained title");
  expect(mutations).toHaveLength(1);
  if (rejected) {
    await expect(editor).toContainText("Your entered edits remain here");
    await expect(editor).toBeVisible();
    return;
  }
  await expect(editor).toContainText("Your newer edits are still here");
  await editor.getByRole("button", { name: "Save event", exact: true }).click();
  await expect(editor).toHaveCount(0);
  await expect.poll(() => mutations.length).toBe(2);
  expect(mutations[1].mutation.save.before).toMatchObject({ id: `shep${mutations[0].request_id.replaceAll("-", "")}`, etag: "v1" });
  expect(mutations[1].mutation.save.after).toMatchObject({ id: mutations[1].mutation.save.before.id, title: "Newer retained title", etag: "v1", remote_url: "created-event" });
});

test("lost delete admission cannot close the editor after the provider rejected it", async ({ page }) => {
  let mutations = 0;
  await page.route("**/api/calendar", async route => {
    const { operation } = route.request().postDataJSON();
    if (operation.kind === "sources") return route.fulfill({ json: { state: "observed", value: { sources: [source] } } });
    if (operation.kind === "events") return route.fulfill({ json: { state: "observed", value: { events: [event()] } } });
    mutations++;
    return route.fulfill({ json: { state: "rejected", error: "Fixture delete refused" } });
  });
  await seed(page); await openCalendar(page);
  await page.getByRole("button", { name: "Planning", exact: true }).click();
  await page.evaluate(async () => {
    const path = "/src/calendar_actions.ts", { BrowserCalendar } = await import(path);
    const admit = BrowserCalendar.prototype.admit; let once = true;
    BrowserCalendar.prototype.admit = async function(input: unknown) {
      const result = await admit.call(this, input);
      if (once) { once = false; throw Error("Committed delete reply lost"); }
      return result;
    };
  });
  const editor = page.getByRole("dialog", { name: "Edit event", exact: true });
  const confirm = async () => {
    await editor.getByRole("button", { name: "Delete event", exact: true }).click();
    await page.getByRole("dialog", { name: "Delete event?", exact: true }).getByRole("button", { name: "Delete this event", exact: true }).click();
  };
  await confirm();
  await expect(editor).toContainText("Committed delete reply lost");
  await expect.poll(async () => (await jobs(page))[0]?.status).toBe("Rejected");
  await confirm();
  await expect(editor).toBeVisible();
  await expect(editor).toContainText("The event remains open");
  await page.screenshot({ path: "../artifacts/web/calendar-delete-refused-light.png", fullPage: true });
  expect(mutations).toBe(1);
  expect((await jobs(page))).toHaveLength(1);
});

test.describe("all-day calendar dates", () => {
  test.use({ timezoneId: "America/Los_Angeles" });
  test("grid, editor and saved dates agree west of UTC", async ({ page }) => {
    const timed = event(), date = timed.start.slice(0, 10);
    let saved = { ...timed, title: "All-day plan", all_day: true, start: `${date}T00:00:00.000Z`, end: new Date(Date.parse(`${date}T00:00:00.000Z`) + 86400000).toISOString() };
    const mutations: any[] = [];
    await page.route("**/api/calendar", async route => {
      const { operation } = route.request().postDataJSON();
      if (operation.kind === "sources") return route.fulfill({ json: { state: "observed", value: { sources: [source] } } });
      if (operation.kind === "events") return route.fulfill({ json: { state: "observed", value: { events: [saved] } } });
      mutations.push(operation);
      saved = { ...operation.mutation.save.after, etag: "v2" };
      return route.fulfill({ json: { state: "acknowledged", value: { receipt: { request_id: operation.request_id, before: operation.mutation.save.before, after: saved } } } });
    });
    await seed(page); await openCalendar(page);
    const cell = page.locator(".day").filter({ has: page.getByRole("button", { name: "All-day plan", exact: true }) });
    await expect(cell.locator(":scope > span")).toHaveText("3");
    await cell.getByRole("button", { name: "All-day plan", exact: true }).click();
    const editor = page.getByRole("dialog", { name: "Edit event", exact: true });
    await expect(editor.getByLabel("Starts on", { exact: true })).toHaveValue(date);
    await expect(editor.getByLabel("Ends on", { exact: true })).toHaveValue(date);
    await expect(editor.locator(":scope > p.muted")).toContainText("3,");
    await editor.getByLabel("Event title", { exact: true }).fill("Same calendar date");
    await editor.getByRole("button", { name: "Save event", exact: true }).click();
    await expect(editor).toHaveCount(0);
    await expect.poll(() => mutations.length).toBe(1);
    expect(mutations[0].mutation.save.after).toMatchObject({ all_day: true, start: `${date}T00:00:00.000Z` });
    await expect(page.locator(".day").filter({ has: page.getByRole("button", { name: "Same calendar date", exact: true }) }).locator(":scope > span")).toHaveText("3");
  });
});

test("queued cancellation does not wait for a held predecessor and retains dependent newer edits", async ({ page }) => {
  let saved = event(), mutations = 0, release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route("**/api/calendar", async route => {
    const { operation } = route.request().postDataJSON();
    if (operation.kind === "sources") return route.fulfill({ json: { state: "observed", value: { sources: [source] } } });
    if (operation.kind === "events") return route.fulfill({ json: { state: "observed", value: { events: [saved] } } });
    mutations++; await held;
    saved = { ...operation.mutation.save.after, etag: "v2" };
    await route.fulfill({ json: { state: "acknowledged", value: { receipt: { request_id: operation.request_id, before: operation.mutation.save.before, after: saved } } } });
  });
  await seed(page); await openCalendar(page);
  for (const [before, title] of [["Planning", "Held predecessor"], ["Held predecessor", "Cancel this queued edit"], ["Cancel this queued edit", "Retained dependent text"]]) {
    await page.getByRole("button", { name: before, exact: true }).click();
    const editor = page.getByRole("dialog", { name: "Edit event", exact: true });
    await editor.getByLabel("Event title", { exact: true }).fill(title);
    await editor.getByRole("button", { name: "Save event", exact: true }).click();
    await expect(editor).toHaveCount(0);
    await expect(page.getByRole("button", { name: title, exact: true })).toBeVisible();
  }
  await expect.poll(() => mutations).toBe(1);
  await page.getByRole("button", { name: "Calendar changes", exact: true }).click();
  const history = page.getByRole("dialog", { name: "Calendar changes", exact: true });
  await history.getByRole("button", { name: "Cancel Cancel this queued edit", exact: true }).click();
  await expect.poll(async () => (await jobs(page)).find(job => job.requested.save?.after.title === "Cancel this queued edit")?.status).toBe("Cancelled");
  expect(mutations).toBe(1);
  release();
  await expect.poll(async () => (await jobs(page)).find(job => job.requested.save?.after.title === "Retained dependent text")?.status).toBe("Rejected");
  expect(mutations).toBe(1);
  await history.getByRole("button", { name: "Refresh Calendar changes", exact: true }).click();
  await history.getByRole("button", { name: "Review saved edits for Retained dependent text", exact: true }).click();
  const retained = page.getByRole("dialog", { name: "Saved Calendar edits", exact: true });
  await expect(retained).toContainText("Retained dependent text");
  await retained.getByRole("button", { name: "Edit retained version", exact: true }).click();
  const editor = page.getByRole("dialog", { name: "Edit event", exact: true });
  await expect(editor.getByLabel("Event title", { exact: true })).toHaveValue("Retained dependent text");
  expect(mutations).toBe(1);
});
