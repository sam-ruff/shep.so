import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { BulkJournal, tabLock, type BulkSweep } from "./bulk_journal";
import { BulkExecutor } from "./bulk_executor";
import { BrowserGroups } from "./bulk_client";
import type { GatewayRepository } from "./provider";
import { FakeLockManager, installFakeLocks } from "./testing/fake_locks";

const profile = "S".repeat(43);
let locks: FakeLockManager;
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
beforeEach(() => {
  locks = installFakeLocks();
  vi.useFakeTimers();
  vi.spyOn(BulkJournal, "inspect").mockResolvedValue([]);
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});
const groups = () =>
  new BrowserGroups(
    { profileId: profile } as GatewayRepository,
    () => undefined,
    1000,
  );
const settled = () => vi.advanceTimersByTimeAsync(0);

test("starting holds this tab's review lock until stop releases it", async () => {
  vi.spyOn(BulkExecutor.prototype, "run").mockResolvedValue({
    steps: 0,
    repairs: 0,
  });
  const g = groups();
  g.start();
  await settled();
  const name = tabLock(profile, g.owner);
  expect((await locks.query()).held.map((lock) => lock.name)).toContain(name);
  g.stop();
  await settled();
  expect((await locks.query()).held.map((lock) => lock.name)).not.toContain(
    name,
  );
});

test("the periodic sweep runs between wakes, waits for a live run and stops with the client", async () => {
  const run = deferred<{ steps: number; repairs: number }>();
  vi.spyOn(BulkExecutor.prototype, "run").mockReturnValue(run.promise);
  const sweep = vi
    .spyOn(BulkExecutor.prototype, "sweep")
    .mockResolvedValue({ retired: 0, rows: 0, more: false });
  const g = groups();
  g.start();
  await vi.advanceTimersByTimeAsync(2500);
  expect(sweep).not.toHaveBeenCalled();
  run.resolve({ steps: 0, repairs: 0 });
  await vi.advanceTimersByTimeAsync(1000);
  expect(sweep).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(1000);
  expect(sweep).toHaveBeenCalledTimes(2);
  g.stop();
  await vi.advanceTimersByTimeAsync(5000);
  expect(sweep).toHaveBeenCalledTimes(2);
});

test("a wake waits for an in-flight sweep, and retired reviews refresh the recovery status", async () => {
  const sweep = deferred<BulkSweep>();
  vi.spyOn(BulkExecutor.prototype, "sweep").mockReturnValue(sweep.promise);
  const run = vi
    .spyOn(BulkExecutor.prototype, "run")
    .mockResolvedValue({ steps: 0, repairs: 0 });
  const inspect = BulkJournal.inspect as unknown as ReturnType<typeof vi.fn>;
  const g = groups();
  const sweeping = g.sweep();
  expect(g.sweep()).toBe(sweeping);
  g.wake();
  await settled();
  expect(run).not.toHaveBeenCalled();
  const before = inspect.mock.calls.length;
  sweep.resolve({ retired: 1, rows: 3, more: false });
  await settled();
  expect(run).toHaveBeenCalledTimes(1);
  await vi.waitFor(() =>
    expect(inspect.mock.calls.length).toBeGreaterThan(before),
  );
  g.stop();
});

test("a sweep refused by another tab's owner is ignored and does not report a group failure", async () => {
  vi.spyOn(BulkExecutor.prototype, "sweep").mockRejectedValue(
    Error("Group work is active in another tab."),
  );
  const g = groups();
  const failures: string[] = [];
  g.addEventListener("failure", (event) =>
    failures.push((event as CustomEvent<string>).detail),
  );
  await g.sweep();
  expect(failures).toEqual([]);
  g.stop();
});
