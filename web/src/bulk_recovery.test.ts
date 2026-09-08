import { afterEach, expect, test, vi } from "vitest";
import { BulkJournal, type BulkAttention } from "./bulk_journal";
import { BulkExecutor } from "./bulk_executor";
import { BrowserGroups, type GroupRecovery } from "./bulk_client";
import type { GatewayRepository } from "./provider";

const profile = "R".repeat(43);
const uncertain: BulkAttention = {
  kind: "uncertain",
  count: 2,
  job: "old-group",
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
function observe() {
  const groups = new BrowserGroups(
    { profileId: profile } as GatewayRepository,
    () => undefined,
  );
  const events: GroupRecovery[] = [];
  groups.addEventListener("attention", (event) =>
    events.push((event as CustomEvent<GroupRecovery>).detail),
  );
  return { groups, events };
}
afterEach(() => vi.restoreAllMocks());

test("recovery observations coalesce repeated requests into one current read and one replacement", async () => {
  const held = deferred<BulkAttention[]>();
  const inspect = vi
    .spyOn(BulkJournal, "inspect")
    .mockReturnValueOnce(held.promise)
    .mockResolvedValue([]);
  const { groups, events } = observe();
  groups.refreshAttention();
  await vi.waitFor(() => expect(inspect).toHaveBeenCalledTimes(1));
  for (let i = 0; i < 100; i++) groups.refreshAttention();
  expect(inspect).toHaveBeenCalledTimes(1);
  held.resolve([uncertain]);
  await vi.waitFor(() => expect(events).toHaveLength(2));
  expect(inspect).toHaveBeenCalledTimes(2);
  expect(events).toEqual([{ entries: [uncertain] }, { entries: [] }]);
  groups.stop();
});

test("a failed status check retains known recovery targets and a later retry clears its error", async () => {
  vi.spyOn(BulkJournal, "inspect")
    .mockResolvedValueOnce([uncertain])
    .mockRejectedValueOnce(Error("Synthetic inspection failure"))
    .mockResolvedValue([]);
  const { groups, events } = observe();
  groups.refreshAttention();
  await vi.waitFor(() => expect(events).toHaveLength(1));
  groups.refreshAttention();
  await vi.waitFor(() => expect(events).toHaveLength(2));
  expect(events[1].entries).toEqual([uncertain]);
  expect(events[1].error).toContain("Refresh their status to retry");
  groups.refreshAttention();
  await vi.waitFor(() => expect(events).toHaveLength(3));
  expect(events[2]).toEqual({ entries: [] });
  groups.stop();
});

test("a cache observation taken during execution cannot report a repair gap after execution finishes", async () => {
  const run = deferred<{ steps: number; repairs: number }>(),
    held = deferred<BulkAttention[]>();
  vi.spyOn(BulkExecutor.prototype, "run").mockReturnValue(run.promise);
  const inspect = vi
    .spyOn(BulkJournal, "inspect")
    .mockReturnValueOnce(held.promise)
    .mockResolvedValue([]);
  const { groups, events } = observe();
  groups.wake();
  groups.refreshAttention();
  await vi.waitFor(() => expect(inspect).toHaveBeenCalledTimes(1));
  run.resolve({ steps: 1, repairs: 1 });
  await new Promise<void>((resolve) => setImmediate(resolve));
  held.resolve([{ kind: "cache", count: 1, job: "already-repaired" }]);
  await vi.waitFor(() => expect(events.at(-1)).toEqual({ entries: [] }));
  expect(inspect).toHaveBeenCalledTimes(2);
  expect(events.flatMap((event) => event.entries)).toEqual([]);
  groups.stop();
});

test("closing the client retires a held recovery read and its queued replacement", async () => {
  const held = deferred<BulkAttention[]>();
  const inspect = vi
    .spyOn(BulkJournal, "inspect")
    .mockReturnValue(held.promise);
  const { groups, events } = observe();
  groups.refreshAttention();
  await vi.waitFor(() => expect(inspect).toHaveBeenCalledTimes(1));
  groups.refreshAttention();
  groups.stop();
  held.resolve([uncertain]);
  await new Promise<void>((resolve) => setImmediate(resolve));
  expect(inspect).toHaveBeenCalledTimes(1);
  expect(events).toEqual([]);
});
