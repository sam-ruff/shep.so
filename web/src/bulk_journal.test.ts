import "fake-indexeddb/auto";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import {
  BulkJournal,
  abandonedReview,
  tabLock,
  type BulkJob,
} from "./bulk_journal";
import { BulkExecutor } from "./bulk_executor";
import { FakeLockManager, installFakeLocks } from "./testing/fake_locks";
import {
  identity,
  operations,
  pages,
  rows,
  stage,
} from "./testing/bulk_fixtures";

let locks: FakeLockManager;
let user = "";
let serial = 0;
beforeEach(() => {
  locks = installFakeLocks();
  user = String.fromCharCode(65 + (serial++ % 26)).repeat(43);
});
afterEach(() => vi.restoreAllMocks());

/** Hold a tab liveness lock until the returned release runs. */
function liveTab(owner: string) {
  let release!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  void locks.request(tabLock(user, owner), () => held);
  return release;
}
const ids = async (journal: BulkJournal) =>
  (await journal.history()).map((job) => job.id).sort();

test("abandoned reviews are cancelled, interrupted, ownerless or owned by a tab whose lock is gone", () => {
  const live = new Set(["tab-live"]);
  const job = (state: BulkJob["state"], owner?: string) =>
    ({ state, owner }) as BulkJob;
  expect(abandonedReview(job("cancelled", "tab-live"), live)).toBe(true);
  expect(abandonedReview(job("interrupted", "tab-live"), live)).toBe(true);
  expect(abandonedReview(job("review"), live)).toBe(true);
  expect(abandonedReview(job("review", "tab-gone"), live)).toBe(true);
  expect(abandonedReview(job("review", "tab-live"), live)).toBe(false);
  expect(abandonedReview(job("ready"), live)).toBe(false);
  expect(abandonedReview(job("preparing"), live)).toBe(false);
});

test("the sweep retires abandoned reviews in bounded strict transactions and leaves live reviews, approved work and receipts alone", async () => {
  const release = liveTab("tab-live");
  await BulkJournal.own(user, async (j) => {
    await stage(j, "live", rows(5), "tab-live");
    await stage(j, "dead", rows(120), "tab-gone");
    await stage(j, "legacy", rows(3));
    await j.cancel(await stage(j, "cancelled", rows(2), "tab-live"));
    async function* broken() {
      yield rows(1);
      throw Error("Synthetic staging loss");
    }
    await j
      .prepare("lost", { kind: "flags", unread: false }, 2, broken(), "epoch-1")
      .catch(() => {});
    const approved = await j.decideCurrent(
      await stage(j, "approved", rows(2), "tab-live"),
      "approve",
      7,
    );
    const item = (await j.claim(approved.id))!;
    await j.settle(approved.id, item.position, item.attempt!, {
      kind: "committed",
      receipt: {
        before: identity(0),
        after: { ...identity(0), starred: true },
      },
      cacheApplied: false,
    });
  });
  const deletes = new Map<IDBTransaction, number>();
  const remove = IDBObjectStore.prototype.delete;
  vi.spyOn(IDBObjectStore.prototype, "delete").mockImplementation(function (
    this: IDBObjectStore,
    key,
  ) {
    if (this.name === "items")
      deletes.set(this.transaction, (deletes.get(this.transaction) ?? 0) + 1);
    return remove.call(this, key);
  });
  // Cancelled and interrupted rows go first, then the first 50 dead-owner rows.
  const first = await BulkJournal.own(user, (j) => j.sweep(3));
  expect(first).toEqual({ retired: 2, rows: 53, more: true });
  await BulkJournal.inspect(user, async (j) => {
    // A partly retired review is fenced so a late approval is refused.
    expect((await j.get("dead")).state).toBe("cancelled");
    expect(await pages(j, "dead")).toBe(70);
    expect(await ids(j)).toEqual(["approved", "dead", "legacy", "live"]);
  });
  const rest = await BulkJournal.own(user, (j) => j.sweep(20));
  expect(rest).toEqual({ retired: 2, rows: 73, more: false });
  expect(Math.max(...deletes.values())).toBe(50);
  await BulkJournal.inspect(user, async (j) => {
    expect(await ids(j)).toEqual(["approved", "live"]);
    expect(await pages(j, "live")).toBe(5);
    expect(await j.get("approved")).toMatchObject({
      state: "ready",
      pendingCache: 1,
      counts: { done: 1, pending: 1 },
    });
    expect((await j.getItem("approved", 0)).receipt?.after.starred).toBe(true);
    expect(await j.pendingCache()).toHaveLength(1);
    expect(await j.attention()).toEqual([
      { kind: "cache", count: 1, job: "approved" },
    ]);
  });
  release();
  await new Promise((resolve) => setImmediate(resolve));
  const gone = await BulkJournal.own(user, (j) => j.sweep(20));
  expect(gone).toEqual({ retired: 1, rows: 5, more: false });
  await BulkJournal.inspect(user, async (j) =>
    expect(await ids(j)).toEqual(["approved"]),
  );
});

test("cancel refuses approved or stale reviews and a cancelled review cannot be approved or claimed", async () => {
  liveTab("tab-live");
  await BulkJournal.own(user, async (j) => {
    const review = await stage(j, "review", rows(2), "tab-live");
    await expect(
      j.cancel({ ...review, revision: review.revision + 1 }),
    ).rejects.toThrow("This group changed");
    const approved = await j.decideCurrent(
      await stage(j, "approved", rows(1), "tab-live"),
      "approve",
      3,
    );
    await expect(j.cancel(approved)).rejects.toThrow("This group changed");
    const cancelled = await j.cancel(review);
    expect(cancelled.state).toBe("cancelled");
    expect(cancelled.runnable).toBe(0);
    expect(await j.cancel(cancelled)).toMatchObject({ state: "cancelled" });
    await expect(j.decideCurrent(cancelled, "approve", 4)).rejects.toThrow(
      "This group changed",
    );
    await expect(j.claim("review")).resolves.toBeNull();
    expect(await j.next()).toMatchObject({ id: "approved" });
  });
});

test("an aborted cleanup transaction leaves every staged row and the review state intact", async () => {
  await BulkJournal.own(user, (j) => stage(j, "dead", rows(120), "tab-gone"));
  const remove = IDBObjectStore.prototype.delete;
  let calls = 0;
  vi.spyOn(IDBObjectStore.prototype, "delete").mockImplementation(function (
    this: IDBObjectStore,
    key,
  ) {
    if (this.name === "items" && ++calls === 30)
      throw Error("Synthetic storage failure");
    return remove.call(this, key);
  });
  await expect(BulkJournal.own(user, (j) => j.sweep(1))).rejects.toThrow(
    "Synthetic storage failure",
  );
  await BulkJournal.inspect(user, async (j) => {
    expect((await j.get("dead")).state).toBe("review");
    expect(await pages(j, "dead")).toBe(120);
  });
});

test("an executor wake retires abandoned reviews before its first step and never sends them to the provider", async () => {
  liveTab("tab-live");
  await BulkJournal.own(user, async (j) => {
    await stage(j, "abandoned", rows(3, 10));
    await j.decideCurrent(
      await stage(j, "approved", rows(1), "tab-live"),
      "approve",
      2,
    );
  });
  const { ops, intents, log } = operations(user);
  intents.clock = 2;
  const executor = new BulkExecutor(user, ops);
  expect(await executor.run()).toEqual({ steps: 1, repairs: 0 });
  expect(log.calls.map((call) => call.id)).toEqual(["m0"]);
  await BulkJournal.inspect(user, async (j) => {
    expect(await ids(j)).toEqual(["approved"]);
    expect((await j.get("approved")).counts.done).toBe(1);
  });
});
