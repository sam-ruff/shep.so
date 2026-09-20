import "fake-indexeddb/auto";
import { afterEach, expect, test, vi } from "vitest";
import { BrowserStore } from "./storage";
import { CalendarRepository } from "./calendar_repository";
import type { ProviderEvent } from "./calendar_actions";

let serial = 0;
const opened: BrowserStore[] = [];
afterEach(() => { vi.restoreAllMocks(); for (const store of opened.splice(0)) store.close(); });
const source = { id: "a", name: "First", read_only: false }, other = { id: "b", name: "Second", read_only: false };
const window = ["2026-09-01T00:00:00Z", "2026-10-01T00:00:00Z"] as const;
const event: ProviderEvent = { id: "event", source_id: "a", title: "Original", start: "2026-09-20T10:00:00Z", end: "2026-09-20T11:00:00Z", location: "", description: "", all_day: false, etag: "v1", remote_url: "event" };
function deferred() { let release!: () => void; const promise = new Promise<void>(resolve => { release = resolve; }); return { promise, release }; }
async function fixture() {
  const store = await BrowserStore.open(`calviews${String(++serial).padStart(35, "0")}`); opened.push(store);
  const journal = store.calendar;
  await journal.saveSources([source, other]);
  await journal.sync(source.id, ...window, [event], await journal.observationRevision());
  await journal.sync(other.id, ...window, [{ ...event, source_id: "b", title: "Second calendar event" }], await journal.observationRevision());
  const request = vi.fn(async (_operation: object): Promise<unknown> => ({ state: "observed", value: { sources: [source, other], events: [event] } }));
  const repository = new CalendarRepository(journal, "tab", request, async (_scope, work) => work(), () => false, () => {});
  await repository.load(); await repository.show(...window, "a");
  const admit = () => journal.admit({ id: crypto.randomUUID(), key: "1:aevent", owner: "tab", mutation: { save: { before: event, after: { ...event, title: "Newer" } } } });
  return { journal, repository, request, admit };
}

test("a delayed view cannot replace a newer projection in the same scope", async () => {
  const { journal, repository, admit } = await fixture(), held = deferred(), started = deferred();
  const view = journal.view.bind(journal);
  vi.spyOn(journal, "view").mockImplementationOnce(async (...args) => {
    const result = await view(...args); started.release(); await held.promise; return result;
  });
  const old = repository.reload(); await started.promise;
  await admit(); await repository.reload();
  expect(repository.events[0].title).toBe("Newer"); expect(repository.pending).toBe(1);
  held.release(); await old;
  expect(repository.events[0].title).toBe("Newer"); expect(repository.pending).toBe(1);
});

test("a delayed summary cannot retire newer pending attention or rows", async () => {
  const { journal, repository, admit } = await fixture(), held = deferred(), started = deferred();
  const summary = journal.summary.bind(journal);
  vi.spyOn(journal, "summary").mockImplementationOnce(async () => {
    const result = await summary(); started.release(); await held.promise; return result;
  });
  const old = repository.reload(); await started.promise;
  const queued = await admit();
  await journal.outcome(queued, "Uncertain", "unknown fixture");
  await repository.reload(); expect(repository.attention).toBe(1);
  held.release(); await old;
  expect(repository.pending).toBe(1); expect(repository.attention).toBe(1); expect(repository.events[0].title).toBe("Newer");
});

test.each(["intent", "sync", "permissions"])("a delayed inspection cannot authorise adoption after newer %s", async change => {
  const { journal, repository, request, admit } = await fixture(), held = deferred(), started = deferred();
  const queued = await admit();
  const running = (await journal.claim(queued, "tab"))!;
  const unknown = await journal.outcome(running, "Uncertain", "lost response");
  request.mockImplementationOnce(async () => {
    started.release(); await held.promise;
    return { state: "observed", value: { current: event } };
  });
  const check = repository.decide(unknown, "check");
  const rejected = expect(check).rejects.toThrow("changed");
  await started.promise;
  if (change === "intent") await journal.admit({ id: crypto.randomUUID(), key: queued.key, owner: "tab", mutation: { save: { before: { ...event, title: "Newer" }, after: { ...event, title: "Newest" } } } });
  else if (change === "sync") await journal.sync(source.id, ...window, [{ ...event, title: "Server changed", etag: "v2" }], await journal.observationRevision());
  else await journal.saveSources([{ ...source, read_only: true }, other]);
  held.release(); await rejected;
  const retained = (await journal.get(unknown.id))!;
  expect(retained.status).toBe("Uncertain");
  expect(retained.observation).toBeUndefined();
  await expect(journal.adoptObserved(retained)).rejects.toThrow("Check the server again");
  expect(request).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ kind: "inspect" }));
});

test("a source-save reply cannot restore the previous calendar after navigation", async () => {
  const { journal, repository, request } = await fixture(), held = deferred(), started = deferred();
  const saveSources = journal.saveSources.bind(journal);
  vi.spyOn(journal, "saveSources").mockImplementationOnce(async (...args) => {
    await saveSources(...args); started.release(); await held.promise;
  });
  const old = repository.refresh(...window, "a"); await started.promise;
  await repository.show(...window, "b");
  expect(repository.events[0].title).toBe("Second calendar event");
  held.release(); await old;
  expect(repository.source).toBe("b"); expect(repository.events[0].title).toBe("Second calendar event");
  expect(request).toHaveBeenCalledTimes(1);
});

test("a late source observation cannot replace permissions after new admission", async () => {
  const { journal, repository, request, admit } = await fixture(), held = deferred(), started = deferred();
  request.mockImplementationOnce(async () => { started.release(); await held.promise; return { state: "observed", value: { sources: [{ ...source, read_only: true }] } }; });
  const old = repository.refresh(...window, "a"); await started.promise;
  await admit(); await repository.reload();
  held.release(); await expect(old).rejects.toThrow("changed");
  expect((await journal.sources()).find(item => item.id === "a")?.read_only).toBe(false);
  expect(repository.events[0].title).toBe("Newer");
});
