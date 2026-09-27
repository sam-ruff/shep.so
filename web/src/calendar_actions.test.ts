import "fake-indexeddb/auto";
import { afterEach, expect, test, vi } from "vitest";
import { BrowserStore } from "./storage";
import { executeCalendar, type ProviderEvent, type CalendarMutation, type CalendarReceipt } from "./calendar_actions";

const opened: BrowserStore[] = [];
let serial = 0;
afterEach(() => { for (const store of opened.splice(0)) store.close(); });
const source = { id: "calendar", name: "Work", read_only: false };
const event: ProviderEvent = { id: "remote", source_id: source.id, title: "Original", start: "2026-09-20T10:00:00Z", end: "2026-09-20T11:00:00Z", location: "Room", description: "Keep exact details", all_day: false, etag: "v1", remote_url: "remote" };
const window = ["2026-09-01T00:00:00Z", "2026-10-01T00:00:00Z"] as const;
const key = `${source.id.length}:${source.id}${event.id}`;
const edit = (before: ProviderEvent, title: string): CalendarMutation => ({ save: { before, after: { ...before, title } } });
async function fixture(seed = true) {
  const profile = `calendar${String(++serial).padStart(35, "0")}`;
  const store = await BrowserStore.open(profile); opened.push(store);
  await store.calendar.saveSources([source]);
  if (seed) await store.calendar.sync(source.id, ...window, [event], await store.calendar.observationRevision());
  return { store, journal: store.calendar, profile };
}
function admission(mutation = edit(event, "First"), eventKey = key) {
  return { id: crypto.randomUUID(), owner: "tab", key: eventKey, mutation };
}

test("admission is durable before provider execution and exact retry returns current state", async () => {
  const { journal } = await fixture();
  const input = admission();
  const job = await journal.admit(input);
  expect(job.status).toBe("Queued");
  expect((await journal.view(source.id, ...window))[0].event.title).toBe("First");
  const running = await journal.claim(job, "tab");
  expect(await journal.admit(input)).toEqual(running);
  expect(await journal.admit({ ...input, owner: "other-live-tab" })).toEqual(running);
  expect((await journal.get(job.id))?.owner).toBe("tab");
  await expect(journal.admit({ ...input, mutation: edit(event, "Different") })).rejects.toThrow("identity");
  await expect(journal.admit(admission(edit({ ...event, etag: "replaced" }, "Stale")))).rejects.toThrow("changed");
});

test("CalDAV action binding survives restart and rejects changed or removed connection", async () => {
  const { store, journal, profile } = await fixture(false);
  const prepared = await journal.admitConnection({ id: "home", endpoint_id: "approved-home", username: "sam" });
  const active = await journal.activateConnection(prepared);
  const caldavSource = { ...source, id: "caldav", connection_id: active.id };
  const caldavEvent = { ...event, source_id: caldavSource.id };
  await journal.saveSources([caldavSource]);
  await journal.sync(caldavSource.id, ...window, [caldavEvent], await journal.observationRevision());
  const caldavKey = `${caldavSource.id.length}:${caldavSource.id}${caldavEvent.id}`;
  const job = await journal.admit(admission(edit(caldavEvent, "First"), caldavKey));
  expect(job.binding).toEqual({ provider: "caldav", connectionId: "home", connectionRevision: active.revision, endpointId: "approved-home" });
  store.close(); opened.splice(opened.indexOf(store), 1);
  const reopened = await BrowserStore.open(profile); opened.push(reopened);
  expect((await reopened.calendar.get(job.id))?.binding).toEqual(job.binding);
  await expect(reopened.calendar.removeConnection(active)).rejects.toThrow("saved changes");
  const changed = await reopened.calendar.activateConnection(active);
  await expect(reopened.calendar.claim(job, "tab")).rejects.toThrow("connection changed");
  await expect(reopened.calendar.observe(job, event, await reopened.calendar.observationRevision())).rejects.toThrow("connection changed");
  expect(changed.revision).toBeGreaterThan(active.revision);
});

test("removed CalDAV connection cannot admit or publish a checked observation", async () => {
  const { journal } = await fixture(false);
  const active = await journal.activateConnection(await journal.admitConnection({ id: "home", endpoint_id: "approved-home", username: "sam" }));
  const caldavSource = { ...source, id: "caldav", connection_id: active.id };
  const caldavEvent = { ...event, source_id: caldavSource.id };
  await journal.saveSources([caldavSource]);
  await journal.sync(caldavSource.id, ...window, [caldavEvent], await journal.observationRevision());
  await journal.removeConnection(active);
  const caldavKey = `${caldavSource.id.length}:${caldavSource.id}${caldavEvent.id}`;
  await expect(journal.admit(admission(edit(caldavEvent, "First"), caldavKey))).rejects.toThrow("Reconnect");
});

test("calendar source capacity applies across Google and CalDAV connections atomically", async () => {
  const { journal } = await fixture(false);
  const googleSources = Array.from({ length: 50 }, (_, index) => ({ ...source, id: `google-${index}` }));
  await journal.saveSources(googleSources);
  const prepared = await journal.admitConnection({ id: "home", endpoint_id: "approved", username: "sam" });
  await expect(journal.saveConnectionSources(prepared.id, [{ ...source, id: "home-source", connection_id: prepared.id }], await journal.observationRevision(), prepared)).rejects.toThrow("50 sources");
  expect(await journal.sources()).toHaveLength(50);
  expect((await journal.connections())[0].status).toBe("Prepared");
});

test("newer edits wait for the exact prior receipt then preserve fields and actual provider version", async () => {
  const { journal } = await fixture();
  const first = await journal.admit(admission());
  const beforeSecond = { ...event, title: "First" };
  const second = await journal.admit(admission(edit(beforeSecond, "Second")));
  expect(await journal.claim(second, "tab")).toBeUndefined();
  const running = (await journal.claim(first, "tab"))!;
  const receipt: CalendarReceipt = { request_id: first.id, before: event, after: { ...beforeSecond, etag: "v2" } };
  const repair = await journal.acknowledge(running, receipt);
  expect((await journal.view(source.id, ...window))[0].event.title).toBe("Second");
  await journal.repair(repair);
  const next = (await journal.claim(second, "tab"))!;
  expect(next.dispatch).toEqual({ save: { before: receipt.after, after: { ...receipt.after, title: "Second" } } });
  expect(next.requested).toEqual(second.requested);
});

test("create receipt rekeys physical identity without duplicating the newer projected event", async () => {
  const { journal } = await fixture(false);
  const provisional = { ...event, id: "local", etag: null, remote_url: null };
  const first = await journal.admit(admission({ save: { before: null, after: provisional } }, "local-key"));
  const second = await journal.admit(admission(edit(provisional, "Newer"), "local-key"));
  const running = (await journal.claim(first, "tab"))!;
  const saved = { ...provisional, id: `shep${first.id.replaceAll("-", "")}`, etag: "v2", remote_url: "actual" };
  await journal.repair(await journal.acknowledge(running, { request_id: first.id, before: null, after: saved }));
  const rows = await journal.view(source.id, ...window);
  expect(rows).toHaveLength(1); expect(rows[0].event.title).toBe("Newer");
  const next = (await journal.claim(second, "tab"))!;
  expect(next.dispatch).toMatchObject({ save: { before: saved, after: { id: saved.id, etag: "v2", description: event.description } } });
});

test("crash after dispatch is uncertain and never replayed; acknowledged cache work is repair only", async () => {
  const { journal, store, profile } = await fixture();
  const job = await journal.admit(admission());
  const running = (await journal.claim(job, "old-tab"))!;
  store.close();
  const reopened = await BrowserStore.open(profile); opened.push(reopened);
  const recovered = await reopened.calendar.recoverAbandoned((await reopened.calendar.get(job.id))!);
  const provider = { mutate: vi.fn() };
  expect(await executeCalendar(reopened.calendar, recovered, "new-tab", provider, () => false)).toEqual(recovered);
  expect(provider.mutate).not.toHaveBeenCalled();
  expect(recovered.status).toBe("Uncertain");
  expect(running.dispatch).toEqual(job.requested);
});

test("receipt survives restart and cache failure without authorising another mutation", async () => {
  const { journal, store, profile } = await fixture();
  const job = await journal.admit(admission());
  const running = (await journal.claim(job, "old-tab"))!;
  await journal.acknowledge(running, { request_id: job.id, before: event, after: { ...event, title: "First", etag: "v2" } });
  store.close();
  const reopened = await BrowserStore.open(profile); opened.push(reopened);
  const provider = { mutate: vi.fn() };
  const result = await executeCalendar(reopened.calendar, (await reopened.calendar.get(job.id))!, "new-tab", provider, () => false);
  expect(result.status).toBe("Succeeded"); expect(provider.mutate).not.toHaveBeenCalled();
  expect((await reopened.calendar.view(source.id, ...window))[0].event.etag).toBe("v2");
});

test("late sync and stale checked recovery cannot replace newer admitted intent", async () => {
  const { journal } = await fixture();
  const beforeSync = await journal.observationRevision();
  const first = await journal.admit(admission());
  await expect(journal.sync(source.id, ...window, [event], beforeSync)).rejects.toThrow("changed");
  const running = (await journal.claim(first, "tab"))!;
  const unknown = await journal.outcome(running, "Uncertain", "lost reply");
  const checked = await journal.observe(unknown, event, await journal.observationRevision());
  const newer = await journal.admit(admission(edit({ ...event, title: "First" }, "Newer")));
  await expect(journal.adoptObserved(checked)).rejects.toThrow("changed");
  expect(await journal.claim(newer, "tab")).toBeUndefined();
  expect((await journal.view(source.id, ...window))[0].event.title).toBe("Newer");
});

test.each(["changed", "absent"])("checked %s state survives restart and adoption without another provider write", async state => {
  const { journal, store, profile } = await fixture();
  const queued = await journal.admit(admission());
  const running = (await journal.claim(queued, "old-tab"))!;
  const unknown = await journal.outcome(running, "Uncertain", "lost reply");
  const observed = state === "absent" ? null : { ...event, title: "Changed on server", etag: "v3" };
  const checked = await journal.observe(unknown, observed, await journal.observationRevision());
  store.close();
  const reopened = await BrowserStore.open(profile); opened.push(reopened);
  const retained = (await reopened.calendar.get(checked.id))!;
  expect(retained.observation).toEqual(checked.observation);
  const adopted = await reopened.calendar.adoptObserved(retained);
  expect(adopted.status).toBe("Dismissed");
  expect(adopted.receipt).toBeUndefined();
  expect(adopted.error).toContain("original provider outcome remains unconfirmed");
  expect((await reopened.calendar.currentEvent(key))?.event ?? null).toEqual(observed);
  const provider = { mutate: vi.fn() };
  expect(await executeCalendar(reopened.calendar, adopted, "new-tab", provider, () => false)).toEqual(adopted);
  expect(provider.mutate).not.toHaveBeenCalled();
});

test("provider refusal is not success and stopped work does not dispatch", async () => {
  const { journal } = await fixture();
  const queued = await journal.admit(admission());
  const provider = { mutate: vi.fn(async () => ({ state: "rejected" as const, error: "Changed version" })) };
  expect(await executeCalendar(journal, queued, "tab", provider, () => true)).toEqual(queued);
  expect(provider.mutate).not.toHaveBeenCalled();
  const result = await executeCalendar(journal, queued, "tab", provider, () => false);
  expect(result.status).toBe("Rejected"); expect(provider.mutate).toHaveBeenCalledOnce();
  expect((await journal.view(source.id, ...window))[0].event.title).toBe("Original");
});

test("source permission and immutable event identity are rechecked before dispatch", async () => {
  const { journal } = await fixture();
  expect(() => journal.admit(admission({ save: { before: event, after: { ...event, source_id: "different" } } }))).toThrow("identity");
  const queued = await journal.admit(admission());
  await journal.saveSources([{ ...source, read_only: true }]);
  const provider = { mutate: vi.fn() };
  const result = await executeCalendar(journal, queued, "tab", provider, () => false);
  expect(result.status).toBe("Rejected"); expect(provider.mutate).not.toHaveBeenCalled();
  expect(await journal.summary()).toEqual({ pending: 1, attention: 1 });
});

test("acknowledged repair cannot overwrite a replacement and checked adoption retains the receipt", async () => {
  const { journal, store } = await fixture();
  const first = await journal.admit(admission());
  const running = (await journal.claim(first, "tab"))!;
  const receipt: CalendarReceipt = { request_id: first.id, before: event, after: { ...event, title: "First", etag: "v2" } };
  const repair = await journal.acknowledge(running, receipt);
  const replacement = { ...event, title: "Newer server edit", etag: "v3" };
  await store.commit([{ store: "calendarEvents", key, value: { key, event: replacement, revision: 900 } }]);
  const provider = { mutate: vi.fn() };
  const retained = await executeCalendar(journal, repair, "tab", provider, () => false);
  expect(retained.status).toBe("Repair"); expect(retained.receipt).toEqual(receipt);
  expect(retained.error).toContain("cached event changed");
  expect(provider.mutate).not.toHaveBeenCalled();
  const checked = await journal.observe(retained, replacement, await journal.observationRevision());
  const adopted = await journal.adoptObserved(checked);
  expect(adopted.status).toBe("Dismissed"); expect(adopted.receipt).toEqual(receipt);
  expect(adopted.error).toContain("acknowledged receipt was retained");
  expect((await journal.view(source.id, ...window))[0].event).toEqual(replacement);
});

test("a resolved predecessor never silently dispatches its retained newer dependent edit", async () => {
  const { journal } = await fixture();
  const first = await journal.admit(admission());
  const newer = await journal.admit(admission(edit({ ...event, title: "First" }, "Retained newer text")));
  const running = (await journal.claim(first, "tab"))!;
  const unknown = await journal.outcome(running, "Uncertain", "lost reply");
  await journal.adoptObserved(await journal.observe(unknown, event, await journal.observationRevision()));
  const provider = { mutate: vi.fn() };
  const result = await executeCalendar(journal, newer, "tab", provider, () => false);
  expect(result.status).toBe("Rejected"); expect(provider.mutate).not.toHaveBeenCalled();
  expect(result.requested).toEqual(newer.requested);
  expect((result.requested as { save: { after: ProviderEvent } }).save.after.title).toBe("Retained newer text");
});

test("queued cancellation wins before claim and retains successors for review; a running claim refuses cancellation", async () => {
  const { journal } = await fixture();
  const first = await journal.admit(admission());
  const newer = await journal.admit(admission(edit({ ...event, title: "First" }, "Retained newer text")));
  const cancelled = await journal.cancel(first);
  expect(cancelled.status).toBe("Cancelled"); expect(cancelled.active).toBe(0);
  const provider = { mutate: vi.fn() };
  expect((await executeCalendar(journal, newer, "tab", provider, () => false)).status).toBe("Rejected");
  expect((await journal.get(newer.id))?.requested).toEqual(newer.requested);
  expect(provider.mutate).not.toHaveBeenCalled();
  await expect(journal.claim(first, "tab")).rejects.toThrow("advanced");
  const latest = await journal.admit(admission(edit(event, "Current")));
  const running = (await journal.claim(latest, "tab"))!;
  await expect(journal.cancel(latest)).rejects.toThrow("advanced");
  await expect(journal.cancel(running)).rejects.toThrow("already started");
  expect((await journal.get(running.id))?.status).toBe("Running");
});
