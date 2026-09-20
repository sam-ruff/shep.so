import "fake-indexeddb/auto";
import { afterEach, expect, test, vi } from "vitest";
import { BrowserStore, openMailDatabase, stores } from "./storage";
import type { Account, RecordMail } from "./provider";
import type { Mailbox } from "./folder_actions";
import { folderSteps, localFolderPlan, type FolderPlan } from "./folder_mutations";
import { resolveMail } from "./sent_cache";
import { removalPreview, reviewStores } from "./account_removal";
import { BulkJournal } from "./bulk_journal";
import { installFakeLocks } from "./testing/fake_locks";
import { rows, stage } from "./testing/bulk_fixtures";

let serial = 0;
const opened: BrowserStore[] = [];
afterEach(() => { for (const store of opened.splice(0)) store.close(); vi.restoreAllMocks(); });
const mailbox: Mailbox = { name: "Projects", delimiter: "/", encoding: "Utf8", selectable: true, no_inferiors: false, non_existent: false, role: null };
const plan = (remove = false): FolderPlan => ({ source: "Projects", action: remove ? "Delete" : { Rename: { name: "Renamed" } }, parent: null, members: [{ path: "Projects", mailbox, listed: true, depth: 0, destination: remove ? null : "Renamed" }] });
async function fixture(count = 103) {
  const profile = `subtree${String(++serial).padStart(36, "0")}`;
  const store = await BrowserStore.open(profile); opened.push(store);
  const account: Account = { id: "work", name: "Work", email: "work@example.test", protocol: "Imap", host: "imap.example.test", port: 993, username: "work", incoming_security: "Tls", incoming_auth: "Password", smtp_host: "smtp.example.test", smtp_port: 465, smtp_username: "work", smtp_security: "Tls", smtp_auth: "Automatic", smtp_separate_password: false, sent_copy: "Automatic", sent_folder: "Sent" };
  await store.commit([{ store: "accounts", key: account.id, value: account }]);
  for (let i = 0; i < count; i++) {
    const id = `m${String(i).padStart(3, "0")}`;
    const value = { core: { id, account_id: account.id, remote_id: `1.${i + 1}`, folder: "Projects", sender: "sender@example.test", recipient: account.email, subject: "Subject stays out of the journal", preview: "Preview stays out", timestamp: i, unread: true, starred: false, attachment_count: 0 }, text: "Private fixture body" };
    await store.commit([{ store: "mail", key: id, value }, { store: "raw", key: id, value: "raw fixture" }]);
  }
  return { store, account, profile, mutations: store.folderMutations };
}

test("frozen preparation and receipt repair use 50-row pages without retaining message content", async () => {
  const { store, mutations } = await fixture();
  const review = await mutations.review("work", plan());
  expect(review.messages).toBe(103);
  const queued = await mutations.prepare(await mutations.admit("rename", "tab", review));
  expect(queued.status).toBe("Queued");
  const frozen = await store.all("folderMembers");
  expect(frozen).toHaveLength(103);
  expect(JSON.stringify(frozen)).not.toContain("Private fixture");
  expect(JSON.stringify(frozen)).not.toContain("Subject stays");
  const running = await store.folderActions.update(queued, { status: "Running" });
  let repair = await mutations.receipt(running, { step: folderSteps(plan())[0], origin: "acknowledged" });
  expect((await store.get<RecordMail>("mail", "m000"))?.core.folder).toBe("Projects");
  repair = await mutations.repair(repair);
  expect(repair.status).toBe("Repair");
  expect(await store.all("folderMembers")).toHaveLength(53);
  expect((await store.get<RecordMail>("mail", "m049"))?.core.folder).toBe("Renamed");
  expect((await store.get<RecordMail>("mail", "m050"))?.core.folder).toBe("Projects");
  repair = await mutations.repair(repair);
  repair = await mutations.repair(repair);
  expect(repair.status).toBe("Succeeded");
  expect(await store.all("folderMembers")).toEqual([]);
  expect((await store.get<RecordMail>("mail", "m102"))?.core.folder).toBe("Renamed");
  expect(repair.mutation?.receipts[0].origin).toBe("acknowledged");
  const canonical = await resolveMail(store, "work:Renamed:1.1");
  expect(canonical?.localId).toBe("m000");
  expect(canonical?.core.folder).toBe("Renamed");
});

test("pending subtree work fences mail intent, cache and account replacement while draft saves remain independent", async () => {
  const { store, mutations, account } = await fixture(1);
  const job = await mutations.admit("delete", "tab", await mutations.review("work", plan(true)));
  await expect(store.intents.register("m000", { starred: true }, "tab")).rejects.toThrow("saved folder change");
  const original = await store.get<RecordMail>("mail", "m000");
  await expect(store.commit([{ store: "mail", key: "m000", value: { ...original, text: "new" } }])).rejects.toThrow("saved folder change");
  await expect(store.commit([{ store: "accounts", key: "work", value: { ...account, host: "other.example.test" } }])).rejects.toThrow("saved folder change");
  await expect(store.commit([{ store: "mailAliases", key: "unproven", value: { alias: "unproven", target: "m000" } }])).rejects.toThrow("saved folder change");
  await expect(store.commit([{ store: "folderCatalogs", key: "work", value: { account: "work", mailboxes: [] } }])).rejects.toThrow("saved folder change");
  await store.commit([{ store: "drafts", key: "draft", value: { id: "draft", accountId: "work", body: "Independent saved text" } }]);
  expect(await store.get("drafts", "draft")).toMatchObject({ body: "Independent saved text" });
  expect((await store.folderActions.get(job.id))?.status).toBe("Preparing");
});

test("new mail and unsettled individual work invalidate destructive admission", async () => {
  const { store, mutations } = await fixture(1);
  const review = await mutations.review("work", plan(true));
  await store.intents.register("m000", { starred: true }, "tab");
  await expect(mutations.admit("delete", "tab", review)).rejects.toThrow("saved mail actions");
  expect(await store.folderActions.get("delete")).toBeUndefined();
});

test("newly acknowledged Sent destinations invalidate a reviewed subtree before admission", async () => {
  const { store, mutations } = await fixture(1);
  const review = await mutations.review("work", plan(true));
  await store.commit([{ store: "mailRoles", key: "work", value: { account: "work", acknowledged: ["Projects"] } }]);
  await expect(mutations.admit("delete", "tab", review)).rejects.toThrow("Special-use folders");
  await expect(mutations.review("work", plan())).rejects.toThrow("Special-use folders");
});

test("interrupted preparation never becomes runnable after a partial membership commit", async () => {
  const { store, mutations } = await fixture();
  const job = await mutations.admit("delete", "tab", await mutations.review("work", plan(true)));
  const add = IDBObjectStore.prototype.add;
  vi.spyOn(IDBObjectStore.prototype, "add").mockImplementation(function (this: IDBObjectStore, value, key) {
    if (this.name === "folderMembers" && value.id === "m050") throw Error("Fixture preparation quota");
    return add.call(this, value, key);
  });
  await expect(mutations.prepare(job)).rejects.toThrow("Fixture preparation quota");
  const saved = await store.folderActions.get(job.id);
  expect(saved).toMatchObject({ status: "Preparing", mutation: { prepared: 50 } });
  expect(await store.all("folderMembers")).toHaveLength(50);
});

test("a physical replacement cannot be deleted by an older folder receipt", async () => {
  const { store, mutations, profile } = await fixture(1);
  const queued = await mutations.prepare(await mutations.admit("delete", "tab", await mutations.review("work", plan(true))));
  const running = await store.folderActions.update(queued, { status: "Running" });
  const receipt = await mutations.receipt(running, { step: folderSteps(plan(true))[0], origin: "acknowledged" });
  const db = await openMailDatabase(profile);
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction("mailMetadata", "readwrite"), request = tx.objectStore("mailMetadata").get("m000");
    request.onsuccess = () => tx.objectStore("mailMetadata").put({ ...request.result, lineage: crypto.randomUUID() }, "m000");
    tx.oncomplete = () => resolve(); tx.onabort = () => reject(tx.error);
  });
  db.close();
  await expect(mutations.repair(receipt)).rejects.toThrow("changed identity");
  expect(await store.get("mail", "m000")).toBeDefined();
  expect((await store.folderActions.get(receipt.id))?.status).toBe("Repair");
});

test("account removal atomically deletes frozen members and rejects delayed receipt writes", async () => {
  const { store, mutations } = await fixture(3);
  const queued = await mutations.prepare(await mutations.admit("delete", "tab", await mutations.review("work", plan(true))));
  const review = removalPreview(await store.snapshot(reviewStores), "work");
  await store.removeAccount(review, true);
  expect(await store.all("folderMembers")).toEqual([]);
  await expect(mutations.receipt(queued, { step: folderSteps(plan(true))[0], origin: "acknowledged" })).rejects.toThrow("fresh review");
});

test("delete retires aliases in bounded batches and keeps the message until their cleanup commits", async () => {
  const { store, mutations } = await fixture(1);
  await store.commit(Array.from({ length: 61 }, (_, i) => ({ store: "mailAliases" as const, key: `alias${i}`, value: { alias: `alias${i}`, target: "m000" } })));
  let job = await mutations.prepare(await mutations.admit("delete", "tab", await mutations.review("work", plan(true))));
  job = await store.folderActions.update(job, { status: "Running" });
  job = await mutations.receipt(job, { step: folderSteps(plan(true))[0], origin: "acknowledged" });
  job = await mutations.repair(job);
  expect(job.status).toBe("Repair");
  expect(await store.all("mailAliases")).toHaveLength(11);
  expect(await store.get("mail", "m000")).toBeDefined();
  expect(await store.get("raw", "m000")).toBeDefined();
  job = await mutations.repair(job);
  expect(job.status).toBe("Succeeded");
  expect(await store.get("mail", "m000")).toBeUndefined();
  expect(await store.get("raw", "m000")).toBeUndefined();
  expect(await store.all("mailAliases")).toEqual([]);
});

test("failed cache transaction retains the receipt and exact source for a cache-only retry", async () => {
  const { store, mutations } = await fixture(2);
  let job = await mutations.prepare(await mutations.admit("rename", "tab", await mutations.review("work", plan())));
  job = await store.folderActions.update(job, { status: "Running" });
  job = await mutations.receipt(job, { step: folderSteps(plan())[0], origin: "acknowledged" });
  const put = IDBObjectStore.prototype.put;
  const failure = vi.spyOn(IDBObjectStore.prototype, "put").mockImplementation(function (this: IDBObjectStore, value, key) {
    if (this.name === "mail" && key === "m001") throw Error("Fixture cache quota");
    return put.call(this, value, key);
  });
  await expect(mutations.repair(job)).rejects.toThrow("Fixture cache quota");
  expect((await store.get<RecordMail>("mail", "m000"))?.core.folder).toBe("Projects");
  expect((await store.folderActions.get(job.id))?.mutation?.receipt).toEqual(job.mutation?.receipt);
  expect(await store.all("folderMembers")).toHaveLength(2);
  failure.mockRestore();
  expect((await mutations.repair(job)).status).toBe("Succeeded");
});

test("local subtree plans preserve hierarchy and reject cycles, collisions and Inbox", () => {
  const catalog = [mailbox, { ...mailbox, name: "Projects/Child" }, { ...mailbox, name: "Other" }, { ...mailbox, name: "INBOX" }];
  expect(folderSteps(localFolderPlan(catalog, "Projects", { Move: { parent: "Other" } }))).toEqual([{ Rename: { source: "Projects", destination: "Other/Projects" } }]);
  expect(folderSteps(localFolderPlan(catalog, "Projects", "Delete"))).toEqual([{ Delete: { source: "Projects/Child" } }, { Delete: { source: "Projects" } }]);
  expect(() => localFolderPlan(catalog, "Projects", { Move: { parent: "Projects/Child" } })).toThrow("outside this subtree");
  expect(() => localFolderPlan(catalog, "Projects", { Rename: { name: "Other" } })).toThrow("destination exists");
  expect(() => localFolderPlan(catalog, "INBOX", "Delete")).toThrow("Refresh this local folder");
});

test("a pending subtree excludes newly staged source and destination groups", async () => {
  installFakeLocks();
  const { mutations, profile } = await fixture(1);
  await mutations.admit("delete", "tab", await mutations.review("work", plan(true)));
  await BulkJournal.own(profile, async journal => {
    await expect(stage(journal, "source", rows(1))).rejects.toThrow("saved folder change");
    const other = rows(1).map(row => ({ ...row, account: "personal", original: { ...row.original!, account: "personal" } }));
    await expect(stage(journal, "destination", other, undefined, { kind: "move", account: "work", folder: "Projects" })).rejects.toThrow("saved folder change");
    const independent = await stage(journal, "independent", other);
    expect(independent.state).toBe("review");
    expect((await journal.get("source")).staged).toBe(0);
  });
});

test("unrelated account cache progress does not invalidate admitted subtree preparation", async () => {
  const { store, mutations, account } = await fixture(1);
  await store.commit([{ store: "accounts", key: "personal", value: { ...account, id: "personal" } }]);
  const job = await mutations.admit("rename", "tab", await mutations.review("work", plan()));
  const mail = await store.get<RecordMail>("mail", "m000");
  await store.commit([{ store: "mail", key: "other", value: { ...mail, core: { ...mail!.core, id: "other", account_id: "personal" } } }]);
  const prepared = await mutations.prepare(job);
  expect(prepared.status).toBe("Queued");
  expect(prepared.mutation?.prepared).toBe(1);
});

test("schema 17 closes older writers and preserves the cache clock and existing receipts", async () => {
  const profile = `upgrade${String(++serial).padStart(36, "0")}`;
  const old = await new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open(`shep.mail.v1.${profile}`, 16);
    request.onupgradeneeded = () => {
      for (const name of stores) if (name !== "folderMembers") request.result.createObjectStore(name);
      request.transaction!.objectStore("cacheState").put({ epoch: "retained-cache", revision: 17, floor: 3 }, "mail");
      request.transaction!.objectStore("folderActions").put({ id: "receipt", account: "work", active: 1, status: "Repair", receiptOrigin: "acknowledged" }, "receipt");
    };
    request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
  });
  let fenced = false;
  old.onversionchange = () => { fenced = true; old.close(); };
  const store = await BrowserStore.open(profile); opened.push(store);
  expect(fenced).toBe(true);
  expect(await store.get("cacheState", "mail")).toEqual({ epoch: "retained-cache", revision: 17, floor: 3 });
  expect(await store.folderActions.get("receipt")).toMatchObject({ status: "Repair", receiptOrigin: "acknowledged" });
  const db = await openMailDatabase(profile);
  expect(db.version).toBe(18);
  expect(db.transaction("folderMembers").objectStore("folderMembers").indexNames.contains("jobFolderId")).toBe(true);
  expect(db.transaction("mailMetadata").objectStore("mailMetadata").indexNames.contains("accountFolderId")).toBe(true);
  db.close();
});

test("checked receipt retirement rejects stale cache or job reviews and keeps newer rows and the receipt", async () => {
  const { store, mutations, profile, account } = await fixture(1);
  await store.commit([{ store: "accounts", key: "personal", value: { ...account, id: "personal" } }]);
  let job = await mutations.prepare(await mutations.admit("rename", "tab", await mutations.review("work", plan())));
  job = await store.folderActions.update(job, { status: "Running" });
  job = await mutations.receipt(job, { step: folderSteps(plan())[0], origin: "acknowledged" });
  const db = await openMailDatabase(profile);
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction(["mailMetadata", "cacheState"], "readwrite");
    const row = tx.objectStore("mailMetadata").get("m000");
    row.onsuccess = () => tx.objectStore("mailMetadata").put({ ...row.result, lineage: "newer-lineage" }, "m000");
    tx.objectStore("cacheState").put({ epoch: "replacement", revision: 1, floor: 0 }, "mail");
    tx.oncomplete = () => resolve(); tx.onabort = () => reject(tx.error);
  });
  db.close();
  await expect(mutations.repair(job)).rejects.toThrow("cache was replaced");
  await expect(mutations.stopCheckedReceipt(job)).rejects.toThrow("Check this acknowledged");
  job = await store.folderActions.update(job, { status: "Checking" });
  const oldCheck = await mutations.checkedReceipt(job, "applied");
  const mail = await store.get<RecordMail>("mail", "m000");
  await store.commit([{ store: "mail", key: "independent", value: { ...mail, core: { ...mail!.core, id: "independent", account_id: "personal" } } }]);
  await expect(mutations.stopCheckedReceipt(oldCheck)).rejects.toThrow("cache changed after");
  job = await store.folderActions.update(oldCheck, { status: "Checking" });
  const current = await mutations.checkedReceipt(job, "changed");
  await expect(mutations.stopCheckedReceipt(oldCheck)).rejects.toThrow("Check this acknowledged");
  const stopped = await mutations.stopCheckedReceipt(current);
  expect(stopped.status).toBe("Dismissed");
  expect(stopped.active).toBe(0);
  expect(stopped.mutation?.receipt?.origin).toBe("acknowledged");
  expect(await store.all("folderMembers")).toHaveLength(1);
  expect(await store.get("mail", "m000")).toEqual(mail);
  const newer = await store.intents.register("m000", { starred: true }, "tab");
  expect(newer.fields).toEqual({ starred: true });
  await expect(mutations.stopCheckedReceipt(current)).rejects.toThrow("Check this acknowledged");
  expect(await store.get("mail", "m000")).toEqual(mail);
});

test("outgoing filing indexes protect frozen settings, selected destinations and receipts without reading MIME", async () => {
  const get = IDBObjectStore.prototype.get;
  vi.spyOn(IDBObjectStore.prototype, "get").mockImplementation(function (this: IDBObjectStore, key) {
    if (this.name === "outgoing") throw Error("Folder review must not read MIME");
    return get.call(this, key);
  });
  for (const kind of ["configured", "selected", "receipt", "descendant"]) {
    const { store, account, mutations } = await fixture(1);
    const outgoing = { id: `submission-${kind}`, draft: { id: `draft-${kind}`, accountId: "work", body: "Private outgoing text" }, account: { ...account, sent_folder: kind === "configured" ? "Projects" : "Sent" }, state: "queued", wire: { raw: "Private exact MIME" }, ...(kind === "selected" ? { sent: { state: "reserved", folder: "Projects", copyAccount: account } } : kind === "receipt" ? { sent: { state: "saved", receipt: { folder: "Projects", remote_id: "1.2" } } } : kind === "descendant" ? { sent: { state: "reserved", folder: "Projects/Future", copyAccount: account } } : {}) };
    await store.commit([{ store: "outgoing", key: outgoing.draft.id, value: outgoing }]);
    await expect(mutations.review("work", plan(true))).rejects.toThrow("Outbox entry owns");
    await store.commit([{ store: "outgoing", key: outgoing.draft.id, value: { ...outgoing, recovery: { action: "returned", draftId: "editable-again" } } }]);
    expect((await mutations.review("work", plan(true))).messages).toBe(1);
  }
});

test("admission rejects changed outgoing metadata and a pending subtree rejects newly reserved filing targets", async () => {
  const { store, account, mutations } = await fixture(1);
  const review = await mutations.review("work", plan());
  const outgoing = { id: "queued:first", draft: { id: "draft", accountId: "work", body: "Retained editable text" }, account, state: "queued" };
  await store.commit([{ store: "outgoing", key: "draft", value: outgoing }]);
  await expect(mutations.admit("rename", "tab", review)).rejects.toThrow("Outbox changed after");
  const job = await mutations.admit("rename", "tab", await mutations.review("work", plan()));
  await expect(store.commit([{ store: "outgoing", key: "second", value: { ...outgoing, draft: { ...outgoing.draft, id: "second" }, account: { ...account, sent_folder: "Projects" } } }])).rejects.toThrow("saved folder change owns this filing destination");
  await expect(store.commit([{ store: "outgoing", key: "draft", value: { ...outgoing, sent: { state: "reserved", folder: "Projects/Future", copyAccount: account } } }])).rejects.toThrow("saved folder change owns this filing destination");
  expect(await store.get("outgoing", "draft")).toEqual(outgoing);
  expect(await store.get("outgoing", "second")).toBeUndefined();
  expect((await store.folderActions.get(job.id))?.status).toBe("Preparing");
});

test("an already reserved filing target can save its acknowledgement without losing receipt ownership", async () => {
  const { store, account, mutations, profile } = await fixture(1);
  await mutations.admit("rename", "tab", await mutations.review("work", plan()));
  const outgoing = { id: "reserved", draft: { id: "draft", accountId: "work", body: "Original text" }, account, state: "sent", sent: { state: "copying", folder: "Projects", copyAccount: account }, wire: { raw: "Original exact MIME" } };
  const db = await openMailDatabase(profile);
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction("outgoing", "readwrite");
    tx.objectStore("outgoing").put(outgoing, "draft");
    tx.oncomplete = () => resolve(); tx.onabort = () => reject(tx.error);
  }); db.close();
  const acknowledged = { ...outgoing, sent: { ...outgoing.sent, state: "saved", receipt: { folder: "Projects", remote_id: "7.9" } } };
  await store.commit([{ store: "outgoing", key: "draft", value: acknowledged }]);
  expect(await store.get("outgoing", "draft")).toEqual(acknowledged);
});

test("local folder admission rechecks children created after the review", async () => {
  const { store, account, mutations } = await fixture(1);
  await store.commit([{ store: "accounts", key: "work", value: { ...account, protocol: "Pop3" } }, { store: "folderCatalogs", key: "work", value: { account: "work", mailboxes: [mailbox] } }]);
  const review = await mutations.review("work", localFolderPlan([mailbox], "Projects", "Delete"));
  await store.commit([{ store: "folderCatalogs", key: "work", value: { account: "work", mailboxes: [mailbox, { ...mailbox, name: "Projects/New" }] } }]);
  await expect(mutations.admit("delete", "tab", review)).rejects.toThrow("local subtree changed");
  expect(await store.folderActions.get("delete")).toBeUndefined();
});
