import "fake-indexeddb/auto";
import { beforeEach, expect, test } from "vitest";
import { BulkJournal, tabLock, type BulkItem } from "./bulk_journal";
import { BulkExecutor } from "./bulk_executor";
import { MutationFailure } from "./model";
import { FakeLockManager, installFakeLocks } from "./testing/fake_locks";
import { identity, operations, rows, stage } from "./testing/bulk_fixtures";

let locks: FakeLockManager;
let user = "";
let serial = 0;
beforeEach(() => {
  locks = installFakeLocks();
  user = `U${String.fromCharCode(65 + (serial++ % 26)).repeat(42)}`;
});
const statuses = (items: BulkItem[]) =>
  Object.fromEntries(items.map((item) => [item.id, item.status]));
async function approve(
  executor: BulkExecutor,
  id: string,
  members: ReturnType<typeof rows>,
  action?: Parameters<typeof stage>[4],
) {
  const job = await BulkJournal.own(user, (j) =>
    stage(j, id, members, undefined, action),
  );
  return executor.decide(id, job.revision, "approve");
}
const view = (id: string) =>
  BulkJournal.inspect(user, async (j) => ({
    job: await j.get(id),
    items: await j.page(id),
  }));

test("Undo of an older group skips fields a newer overlapping group owns, and each Undo restores its own physical baseline", async () => {
  const { ops, log } = operations(user);
  const executor = new BulkExecutor(user, ops);
  await approve(executor, "flag", rows(2));
  await executor.run();
  const flagged = rows(1).map((row) => ({
    ...row,
    original: { ...identity(0), starred: true },
  }));
  await approve(executor, "unflag", flagged, { kind: "flags", starred: false });
  await executor.run();
  const first = await view("flag");
  await executor.decide("flag", first.job.revision, "undo");
  await executor.run();
  const undone = await view("flag");
  expect(statuses(undone.items)).toEqual({ m0: "skipped", m1: "restored" });
  expect(undone.items[0].error).toContain("A newer choice owns these fields");
  expect(undone.items[1].inverse?.after.starred).toBe(false);
  expect(log.calls).toEqual([
    { id: "m0", fields: { starred: true } },
    { id: "m1", fields: { starred: true } },
    { id: "m0", fields: { starred: false } },
    { id: "m1", fields: { starred: false } },
  ]);
  const second = await view("unflag");
  await executor.decide("unflag", second.job.revision, "undo");
  await executor.run();
  const restored = await view("unflag");
  expect(statuses(restored.items)).toEqual({ m0: "restored" });
  expect(restored.items[0].inverse).toMatchObject({
    before: { starred: false },
    after: { starred: true },
  });
  expect(log.calls.at(-1)).toEqual({ id: "m0", fields: { starred: true } });
});

test("Undo of a group approved behind an earlier group cancels its unsent work without a provider call", async () => {
  let release!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  let holding: (() => void) | undefined;
  const opened = new Promise<void>((resolve) => {
    holding = resolve;
  });
  const { ops, log } = operations(user, async (_id, fields, expected) => {
    if (log.calls.length === 1) {
      holding?.();
      await held;
    }
    return {
      receipt: { before: expected, after: { ...expected, ...fields } },
      cacheApplied: true,
      applied: fields,
    };
  });
  const executor = new BulkExecutor(user, ops);
  await approve(executor, "first", rows(2));
  // The second review stays frozen under this tab's live lock until the
  // running owner is asked to approve it behind the first group.
  void locks.request(tabLock(user, "tab-live"), () => new Promise(() => {}));
  const review = await BulkJournal.own(user, (j) =>
    stage(j, "second", rows(2, 10), "tab-live"),
  );
  const run = executor.run();
  await opened;
  const queued = await executor.decide("second", review.revision, "approve");
  expect(queued.state).toBe("ready");
  const undone = await executor.decide("second", queued.revision, "undo");
  expect(undone.undo).toBe(true);
  release();
  expect(await run).toEqual({ steps: 2, repairs: 0 });
  const first = await view("first"),
    second = await view("second");
  expect(first.job.counts.done).toBe(2);
  expect(second.job.counts).toMatchObject({ pending: 2, done: 0, restored: 0 });
  expect(second.job.runnable).toBe(0);
  expect(log.calls.map((call) => call.id)).toEqual(["m0", "m1"]);
  expect(await executor.run()).toEqual({ steps: 0, repairs: 0 });
  expect(log.calls).toHaveLength(2);
});

test("after a partial failure Undo restores only acknowledged messages and the failed step is neither retried nor repeated", async () => {
  const { ops, log } = operations(user, (id, fields, expected) => {
    if (id === "m1") throw Error("Synthetic server rejection");
    return {
      receipt: { before: expected, after: { ...expected, ...fields } },
      cacheApplied: true,
      applied: fields,
    };
  });
  const executor = new BulkExecutor(user, ops);
  await approve(executor, "archive", rows(3), {
    kind: "move",
    folder: "Archive",
    account: null,
  });
  await executor.run();
  const ran = await view("archive");
  expect(statuses(ran.items)).toEqual({
    m0: "done",
    m1: "failed",
    m2: "done",
  });
  await executor.decide("archive", ran.job.revision, "undo");
  await executor.run();
  const undone = await view("archive");
  expect(statuses(undone.items)).toEqual({
    m0: "restored",
    m1: "failed",
    m2: "restored",
  });
  await expect(
    executor.retry("archive", undone.job.revision, 1),
  ).rejects.toThrow("This group changed");
  expect(log.calls.map((call) => `${call.id}:${call.fields.folder}`)).toEqual([
    "m0:Archive",
    "m1:Archive",
    "m2:Archive",
    "m0:INBOX",
    "m2:INBOX",
  ]);
});

test("an unconfirmed step pauses the group; Undo and Resume never repeat it and only an explicit checked review retires it", async () => {
  const { ops, intents, log } = operations(user, (id, fields, expected) => {
    if (id === "m0") throw new MutationFailure("Synthetic lost provider reply");
    return {
      receipt: { before: expected, after: { ...expected, ...fields } },
      cacheApplied: true,
      applied: fields,
    };
  });
  const executor = new BulkExecutor(user, ops);
  await approve(executor, "flag", rows(2));
  await executor.run();
  const paused = await view("flag");
  expect(paused.job.paused).toBe(true);
  expect(statuses(paused.items)).toEqual({ m0: "uncertain", m1: "pending" });
  await executor.decide("flag", paused.job.revision, "undo");
  await executor.run();
  const undone = await view("flag");
  await executor.decide("flag", undone.job.revision, "resume");
  await executor.run();
  const resumed = await view("flag");
  expect(statuses(resumed.items)).toEqual({ m0: "uncertain", m1: "pending" });
  expect(log.calls).toHaveLength(1);
  await expect(executor.retry("flag", resumed.job.revision, 0)).rejects.toThrow(
    "This group changed",
  );
  await executor.resolve("flag", resumed.job.revision, 0);
  const resolved = await view("flag");
  expect(statuses(resolved.items)).toEqual({ m0: "skipped", m1: "pending" });
  expect(resolved.items[0].error).toContain("accepted the current state");
  expect(intents.finished.map(([lease, status]) => [lease.id, status])).toEqual(
    [["m0", "failed"]],
  );
  expect(await executor.run()).toEqual({ steps: 0, repairs: 0 });
  expect(log.calls).toHaveLength(1);
});
