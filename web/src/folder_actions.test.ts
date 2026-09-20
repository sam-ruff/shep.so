import "fake-indexeddb/auto";
import { afterEach, expect, test, vi } from "vitest";
import { BrowserStore } from "./storage";
import { executeFolderCreation, folderConnection, type Mailbox, type FolderProvider } from "./folder_actions";
import type { Account } from "./provider";
import { removalPreview, reviewStores } from "./account_removal";

const stores: BrowserStore[] = [];
let serial = 0;
afterEach(() => { for (const store of stores.splice(0)) store.close(); });
async function fixture(protocol: Account["protocol"] = "Imap") {
  const store = await BrowserStore.open(`folders${String(++serial).padStart(36, "0")}`);
  stores.push(store);
  const account: Account = { id: "work", name: "Work", email: "work@example.test", protocol, host: "imap.example.test", port: 993, username: "work", incoming_security: "Tls", incoming_auth: "Password", smtp_host: "smtp.example.test", smtp_port: 465, smtp_username: "work", smtp_security: "Tls", smtp_auth: "Automatic", smtp_separate_password: false, sent_copy: "Automatic", sent_folder: "Sent" };
  await store.commit([{ store: "accounts", key: account.id, value: account }]);
  const input = { id: "job", account: account.id, connection: folderConnection(account), name: "Projects", parent: null, owner: "tab" };
  return { store, account, input, journal: store.folderActions };
}
const target: Mailbox = { name: "Projects", delimiter: "/", encoding: "ImapUtf7", selectable: true, no_inferiors: false, non_existent: false, role: null };

test("local admission is idempotent and contains no provider or credential data", async () => {
  const { input, journal } = await fixture();
  const first = await journal.admit(input);
  expect(first).toMatchObject({ status: "Queued", revision: 1, active: 1 });
  expect(await journal.admit(input)).toEqual(first);
  await expect(journal.admit({ ...input, name: "Other" })).rejects.toThrow("identity");
  expect((await journal.page()).rows).toEqual([first]);
  expect(JSON.stringify(first)).not.toContain("password");
});

test("acknowledgement survives cache failure and stale replies cannot replace a newer decision", async () => {
  const { input, journal, store } = await fixture();
  const queued = await journal.admit(input);
  const running = await journal.update(queued, { status: "Running", target });
  const repair = await journal.update(running, { status: "Repair", receipt: target });
  await expect(journal.finish(repair, { ...target, name: "Wrong" })).rejects.toThrow("not confirmed");
  expect(await journal.get(input.id)).toEqual(repair);
  expect(await store.all("folderCatalogs")).toEqual([]);
  const completed = await journal.finish(repair, target);
  expect(completed.status).toBe("Succeeded");
  await expect(journal.update(repair, { status: "Uncertain" })).rejects.toThrow("changed");
  expect(await store.all("folderCatalogs")).toEqual([{ account: input.account, mailboxes: [target] }]);
  expect((await journal.page()).rows).toEqual([]);
  expect((await journal.page(undefined, true)).rows).toEqual([completed]);
});

test("POP3 creation commits its local catalogue and receipt atomically", async () => {
  const { input, journal, store } = await fixture("Pop3");
  const parent = await journal.admit(input);
  expect(parent.status).toBe("Succeeded");
  expect(parent.receipt?.encoding).toBe("Utf8");
  expect(parent.receiptOrigin).toBe("local");
  expect((await journal.admit({ ...input, id: "already-present" })).receiptOrigin).toBe("observed");
  const child = await journal.admit({ ...input, id: "child", parent: "Projects", name: "Active" });
  expect(child.receipt?.name).toBe("Projects/Active");
  await expect(journal.admit({ ...input, id: "bad", name: "one/two" })).rejects.toThrow("one folder");
  expect(await journal.get("bad")).toBeUndefined();
  expect((await store.all<any>("folderCatalogs"))[0].mailboxes).toHaveLength(2);
});

test("folder review is included in account removal and removed accounts reject delayed receipts", async () => {
  const { input, journal, store } = await fixture();
  const job = await journal.admit(input);
  const snapshot = await store.snapshot(reviewStores);
  expect(removalPreview(snapshot, input.account).changes).toBe(1);
  await store.commit([{ store: "removedAccounts", key: input.account, value: { id: input.account } }]);
  await expect(journal.update(job, { status: "Running", target })).rejects.toThrow("removed");
  expect((await journal.get(input.id))?.status).toBe("Queued");
});

test("a changed account cannot dispatch an old folder request but may stop local tracking", async () => {
  const { input, journal, store, account } = await fixture();
  const queued = await journal.admit(input);
  await store.commit([{ store: "accounts", key: account.id, value: { ...account, host: "replacement.example.test" } }]);
  await expect(journal.update(queued, { status: "Running", target })).rejects.toThrow("connection changed");
  const stopped = await journal.update(queued, { status: "Dismissed" });
  expect(stopped.connection).toBe(input.connection);
  expect(stopped.status).toBe("Dismissed");
});

test("bounded pages retain every pending job without mixing completed history", async () => {
  const { input, journal } = await fixture();
  for (let i = 0; i < 23; i++) await journal.admit({ ...input, id: `job-${String(i).padStart(2, "0")}` });
  const first = await journal.page();
  const last = await journal.page(first.next);
  expect(first.rows).toHaveLength(20);
  expect(last.rows).toHaveLength(3);
  expect(last.next).toBeUndefined();
  expect(new Set([...first.rows, ...last.rows].map(row => row.id)).size).toBe(23);
  expect((await journal.page(undefined, true)).rows).toEqual([]);
});

test("provider acknowledgement is saved before inspection and cache recovery never repeats CREATE", async () => {
  const { input, journal } = await fixture();
  const job = await journal.admit(input);
  const provider: FolderProvider = {
    plan: vi.fn(async () => target),
    create: vi.fn(async () => ({ state: "acknowledged" as const, target })),
    inspect: vi.fn(async () => {
      expect((await journal.get(job.id))?.status).toBe("Repair");
      expect((await journal.get(job.id))?.receipt).toEqual(target);
      throw Error("Catalogue unavailable");
    }),
  };
  const repair = await executeFolderCreation(journal, job, provider, () => false);
  expect(repair).toMatchObject({ status: "Repair", error: "Catalogue unavailable" });
  expect(repair.receiptOrigin).toBe("acknowledged");
  provider.inspect = vi.fn(async () => target);
  expect((await executeFolderCreation(journal, repair, provider, () => false)).status).toBe("Succeeded");
  expect(provider.create).toHaveBeenCalledTimes(1);
  expect(provider.plan).toHaveBeenCalledTimes(1);
});

test("unknown CREATE is not retried and only a checked absence permits explicit retry", async () => {
  const { input, journal } = await fixture();
  const provider: FolderProvider = { plan: vi.fn(async () => target), create: vi.fn(async () => { throw Error("Lost response"); }), inspect: vi.fn(async () => null) };
  const unknown = await executeFolderCreation(journal, await journal.admit(input), provider, () => false);
  expect(unknown.status).toBe("Uncertain");
  expect(await executeFolderCreation(journal, unknown, provider, () => false)).toEqual(unknown);
  expect(provider.create).toHaveBeenCalledTimes(1);
  const checked = await journal.update(unknown, { status: "Checking" });
  const absent = await executeFolderCreation(journal, checked, provider, () => false);
  expect(absent.status).toBe("Rejected");
  expect(provider.inspect).toHaveBeenCalledWith(target);
  expect(provider.create).toHaveBeenCalledTimes(1);
});

test("an existing folder and a recovered unknown attempt never acquire creator provenance", async () => {
  const { input, journal } = await fixture();
  const provider: FolderProvider = { plan: vi.fn(async () => target), create: vi.fn(async () => ({ state: "observed" as const, mailbox: target })), inspect: vi.fn(async () => target) };
  const observed = await executeFolderCreation(journal, await journal.admit(input), provider, () => false);
  expect(observed.receiptOrigin).toBe("observed");
  const unknown = await journal.update(await journal.admit({ ...input, id: "unknown" }), { status: "Checking", target });
  expect((await executeFolderCreation(journal, unknown, provider, () => false)).receiptOrigin).toBe("observed");
});

test("failed receipt persistence leaves a started request that cannot execute again", async () => {
  const { input, journal } = await fixture();
  const job = await journal.admit(input);
  const update = journal.update.bind(journal);
  const fail = vi.spyOn(journal, "update").mockImplementation((expected, changes) => {
    if (changes.status === "Repair") return Promise.reject(Error("Receipt storage unavailable"));
    return update(expected, changes);
  });
  const provider: FolderProvider = { plan: vi.fn(async () => target), create: vi.fn(async () => ({ state: "acknowledged" as const, target })), inspect: vi.fn() };
  await expect(executeFolderCreation(journal, job, provider, () => false)).rejects.toThrow("Receipt storage unavailable");
  fail.mockRestore();
  const started = (await journal.get(job.id))!;
  expect(started.status).toBe("Running");
  expect(await executeFolderCreation(journal, started, provider, () => false)).toEqual(started);
  expect(provider.create).toHaveBeenCalledTimes(1);
  expect(provider.inspect).not.toHaveBeenCalled();
});

test("read failures and stop before dispatch preserve a safely queued request", async () => {
  const { input, journal } = await fixture();
  const provider: FolderProvider = { plan: vi.fn(async () => { throw Error("Offline"); }), create: vi.fn(), inspect: vi.fn() };
  const waiting = await executeFolderCreation(journal, await journal.admit(input), provider, () => false);
  expect(waiting.status).toBe("Waiting");
  expect(provider.create).not.toHaveBeenCalled();
  provider.plan = vi.fn(async () => target);
  let stopped = false;
  provider.plan = vi.fn(async () => { stopped = true; return target; });
  const held = await executeFolderCreation(journal, waiting, provider, () => stopped);
  expect(held.status).toBe("Waiting");
  expect(held.target).toEqual(target);
  expect(provider.create).not.toHaveBeenCalled();
});
