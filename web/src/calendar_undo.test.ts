import "fake-indexeddb/auto";
import { afterEach, expect, test, vi } from "vitest";
import { BrowserStore } from "./storage";
import { calendarAfter, calendarBefore, executeCalendar, type CalendarAction, type CalendarMutation, type ProviderEvent } from "./calendar_actions";

let serial = 0;
const opened: BrowserStore[] = [];
afterEach(() => { vi.restoreAllMocks(); for (const store of opened.splice(0)) store.close(); });
const source = { id: "calendar", name: "Work", read_only: false };
const original: ProviderEvent = { id: "original", source_id: source.id, title: "Original", start: "2026-09-20T10:00:00Z", end: "2026-09-20T11:00:00Z", location: "Room", description: "Exact details", all_day: false, etag: "v1", remote_url: "original" };
const window = ["2026-09-01T00:00:00Z", "2026-10-01T00:00:00Z"] as const;
const key = `${source.id.length}:${source.id}${original.id}`;
async function fixture(kind: "create" | "edit" | "delete" = "edit") {
  const profile = `calundo${String(++serial).padStart(36, "0")}`;
  const store = await BrowserStore.open(profile); opened.push(store);
  const journal = store.calendar;
  await journal.saveSources([source]);
  if (kind !== "create") await journal.sync(source.id, ...window, [original], await journal.observationRevision());
  const mutation: CalendarMutation = kind === "delete" ? { delete: { before: original } } : {
    save: { before: kind === "create" ? null : original, after: kind === "create" ? { ...original, etag: null, remote_url: null } : { ...original, title: "Edited" } },
  };
  const admitted = await journal.admit({ id: crypto.randomUUID(), key, owner: "tab", mutation });
  const running = (await journal.claim(admitted, "tab"))!;
  const desired = calendarAfter(mutation);
  const after = desired ? { ...desired, id: kind === "create" ? `shep${admitted.id.replaceAll("-", "")}` : original.id, etag: "v2", remote_url: "actual-resource" } : null;
  const job = await journal.repair(await journal.acknowledge(running, { request_id: running.id, before: calendarBefore(mutation), after }));
  return { store, journal, profile, job, after };
}

test.each(["create", "edit"] as const)("Undo of %s admits an exact inverse and projects before any provider work", async kind => {
  const { journal, job, after } = await fixture(kind);
  const id = crypto.randomUUID();
  const inverse = await journal.undo(job, id, "tab");
  expect(inverse.undoOf).toBe(job.id); expect(inverse.status).toBe("Queued");
  expect((await journal.get(job.id))?.undoAction).toBe(id);
  expect(calendarBefore(inverse.requested)).toEqual(after);
  const rows = await journal.view(source.id, ...window);
  if (kind === "create") expect(rows).toHaveLength(0);
  else {
    expect(rows).toHaveLength(1);
    expect(rows[0].event).toMatchObject({ title: original.title, location: original.location, description: original.description });
    expect(rows[0].event.etag).toBe(after?.etag);
  }
  const claimed = (await journal.claim(inverse, "tab"))!;
  expect(claimed.dispatch).toEqual(inverse.requested);
});

test("delete Undo cannot recreate an incomplete event from portable fields", async () => {
  const { journal, job } = await fixture("delete");
  const id = crypto.randomUUID();
  await expect(journal.undo(job, id, "tab")).rejects.toThrow("verified provider restoration receipt");
  expect(await journal.get(id)).toBeUndefined();
  expect((await journal.get(job.id))?.undoAction).toBeUndefined();
  expect(await journal.view(source.id, ...window)).toEqual([]);
});

test("lost Undo admission replies and another tab cannot duplicate or take over the inverse", async () => {
  const { journal, store, profile, job } = await fixture();
  const id = crypto.randomUUID(), inverse = await journal.undo(job, id, "tab");
  store.close();
  const reopened = await BrowserStore.open(profile); opened.push(reopened);
  expect(await reopened.calendar.undo(job, id, "other-tab")).toEqual(inverse);
  await expect(reopened.calendar.undo(job, crypto.randomUUID(), "other-tab")).rejects.toThrow("already saved");
  expect((await reopened.calendar.page()).rows).toHaveLength(1);
  const running = (await reopened.calendar.claim(inverse, "tab"))!;
  await reopened.calendar.recoverAbandoned(running);
  const retained = (await reopened.calendar.undo(job, id, "other-tab"))!;
  expect(retained.status).toBe("Uncertain");
  const provider = { mutate: vi.fn() };
  await executeCalendar(reopened.calendar, retained, "other-tab", provider, () => false);
  expect(provider.mutate).not.toHaveBeenCalled();
});

test.each(["intent", "sync", "permissions"])("Undo cannot overwrite newer %s", async change => {
  const { journal, job, after } = await fixture();
  if (change === "intent") await journal.admit({ id: crypto.randomUUID(), key, owner: "tab", mutation: { save: { before: after, after: { ...after!, title: "Newer choice" } } } });
  else if (change === "sync") await journal.sync(source.id, ...window, [{ ...after!, title: "Other device", etag: "v3" }], await journal.observationRevision());
  else await journal.saveSources([{ ...source, read_only: true }]);
  await expect(journal.undo(job, crypto.randomUUID(), "tab")).rejects.toThrow();
  expect((await journal.get(job.id))?.undoAction).toBeUndefined();
  expect((await journal.view(source.id, ...window))[0].event.title).toBe(change === "intent" ? "Newer choice" : change === "sync" ? "Other device" : "Edited");
});

test("inverse admission failure rolls back the original decision and can retry the same identity", async () => {
  const { journal, job } = await fixture();
  const put = IDBObjectStore.prototype.put;
  const failure = vi.spyOn(IDBObjectStore.prototype, "put").mockImplementation(function (this: IDBObjectStore, value: CalendarAction, key?: IDBValidKey) {
    if (this.name === "calendarActions" && value.undoOf) throw Error("Local storage full");
    return put.call(this, value, key);
  });
  const id = crypto.randomUUID();
  await expect(journal.undo(job, id, "tab")).rejects.toThrow("Local storage full");
  expect((await journal.get(job.id))?.undoAction).toBeUndefined();
  expect(await journal.get(id)).toBeUndefined();
  expect((await journal.view(source.id, ...window))[0].event.title).toBe("Edited");
  failure.mockRestore();
  expect((await journal.undo(job, id, "tab")).status).toBe("Queued");
});

test("a refused inverse restores the acknowledged result without erasing the original receipt", async () => {
  const { journal, job, after } = await fixture();
  const inverse = await journal.undo(job, crypto.randomUUID(), "tab");
  const provider = { mutate: vi.fn(async () => ({ state: "rejected" as const, error: "Version changed" })) };
  expect((await executeCalendar(journal, inverse, "tab", provider, () => false)).status).toBe("Rejected");
  expect((await journal.view(source.id, ...window))[0].event).toEqual(after);
  expect((await journal.get(job.id))?.receipt).toEqual(job.receipt);
});
