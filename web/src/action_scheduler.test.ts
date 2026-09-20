import { expect, test, vi } from "vitest";
import { ActionWake, runAccountActions, type AccountWork, type ActionSource } from "./action_scheduler";

const deferred = () => {
  let resolve!: () => void;
  const promise = new Promise<void>(done => { resolve = done; });
  return { promise, resolve };
};
function source(rows: AccountWork[]): ActionSource {
  return { page: async after => {
    const found = rows.filter(row => !after || row.id > after).slice(0, 21);
    return { rows: found.slice(0, 20), next: found.length > 20 ? found[19].id : undefined };
  } };
}

test("held mail and its same-account backlog do not starve a folder on another account", async () => {
  const held = deferred(), folder = deferred();
  const started: string[] = [];
  const mail = Array.from({ length: 45 }, (_, i) => ({ id: String(i).padStart(3, "0"), account: "held", run: async () => { started.push(`mail-${i}`); if (!i) await held.promise; } }));
  const run = runAccountActions([source(mail), source([{ id: "folder", account: "other", run: async () => { started.push("folder"); folder.resolve(); } }])], new ActionWake(), () => false);
  await folder.promise;
  expect(started).toEqual(["mail-0", "folder"]);
  held.resolve(); await run;
  expect(started).toHaveLength(46);
});

test("new local admission wakes a pass waiting on held provider work", async () => {
  const held = deferred(), admitted = deferred(), wake = new ActionWake();
  const waiting = vi.spyOn(wake, "wait");
  const folders: AccountWork[] = [];
  const run = runAccountActions([source([{ id: "mail", account: "work", run: () => held.promise }]), source(folders)], wake, () => false);
  await vi.waitFor(() => expect(waiting).toHaveBeenCalled());
  folders.push({ id: "folder", account: "personal", run: async () => { admitted.resolve(); } });
  wake.notify();
  await admitted.promise;
  held.resolve(); await run;
});

test("held folder excludes only its account and stop drains active work without claiming siblings", async () => {
  const held = deferred(), started = deferred(), wake = new ActionWake();
  let stopped = false;
  const sibling = vi.fn(async () => {}), unrelated = vi.fn(async () => {});
  const run = runAccountActions([source([{ id: "a", account: "other", run: unrelated }]), source([
    { id: "a", account: "folder", run: async () => { started.resolve(); await held.promise; } },
    { id: "b", account: "folder", run: sibling },
  ])], wake, () => stopped);
  await started.promise;
  stopped = true; wake.notify();
  expect(unrelated).toHaveBeenCalledOnce(); expect(sibling).not.toHaveBeenCalled();
  let finished = false; void run.then(() => { finished = true; });
  await Promise.resolve(); expect(finished).toBe(false);
  held.resolve(); await run;
  expect(sibling).not.toHaveBeenCalled();
});

test("a retained Waiting or cache-repair result is attempted once per wake pass", async () => {
  const work = vi.fn(async () => {});
  await runAccountActions([source([{ id: "waiting", account: "a", run: work }]), source([{ id: "repair", account: "b", run: work }])], new ActionWake(), () => false);
  expect(work).toHaveBeenCalledTimes(2);
});

test("independent accounts still respect the shared four-request limit", async () => {
  const held = deferred();
  let active = 0, peak = 0, finished = 0;
  const work = Array.from({ length: 12 }, (_, i) => ({
    id: String(i).padStart(2, "0"), account: `account-${i}`,
    run: async () => { active++; peak = Math.max(peak, active); await held.promise; active--; finished++; },
  }));
  const run = runAccountActions([source(work.slice(0, 6)), source(work.slice(6))], new ActionWake(), () => false);
  await vi.waitFor(() => expect(active).toBe(4));
  held.resolve(); await run;
  expect(peak).toBe(4); expect(finished).toBe(12);
});

test("a ready folder receives a slot before draining a page of distinct mail accounts", async () => {
  const held = deferred(), folder = deferred();
  const started: string[] = [];
  const mail = Array.from({ length: 25 }, (_, i) => ({
    id: String(i).padStart(2, "0"), account: `mail-${i}`,
    run: async () => { started.push(`mail-${i}`); await held.promise; },
  }));
  const run = runAccountActions([source(mail), source([{ id: "folder", account: "folders", run: async () => { started.push("folder"); folder.resolve(); } }])], new ActionWake(), () => false);
  await folder.promise;
  expect(started.indexOf("folder")).toBe(1);
  held.resolve(); await run;
});

test("a newly admitted folder can take the next released slot ahead of the remaining mail page", async () => {
  const wake = new ActionWake(), folder = deferred();
  const gates = Array.from({ length: 10 }, deferred), folders: AccountWork[] = [];
  const started: string[] = [];
  const mail = gates.map((gate, i) => ({ id: String(i), account: `mail-${i}`, run: async () => { started.push(`mail-${i}`); await gate.promise; } }));
  const run = runAccountActions([source(mail), source(folders)], wake, () => false);
  await vi.waitFor(() => expect(started).toHaveLength(4));
  folders.push({ id: "new-folder", account: "folders", run: async () => { started.push("folder"); folder.resolve(); } });
  wake.notify();
  gates[0].resolve();
  await folder.promise;
  expect(started.indexOf("folder")).toBe(4);
  for (const gate of gates) gate.resolve();
  await run;
});
