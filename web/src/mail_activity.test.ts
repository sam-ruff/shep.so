import "fake-indexeddb/auto";
import { afterEach, expect, test, vi } from "vitest";
import { BrowserStore, openMailDatabase } from "./storage";
import { actionId, queuedActionFields } from "./mail_activity";
import { removalPreview, reviewStores } from "./account_removal";

let serial = 0;
const opened: BrowserStore[] = [];
afterEach(() => { for (const store of opened.splice(0)) store.close(); vi.restoreAllMocks(); });
async function setup() {
  const profile = `activity${String(++serial).padStart(35, "0")}`;
  const store = await BrowserStore.open(profile); opened.push(store);
  await store.commit([
    { store: "accounts", key: "work", value: { id: "work", email: "work@example.test" } },
    { store: "mail", key: "m1", value: { core: { id: "m1", account_id: "work", remote_id: "1", folder: "INBOX", unread: true, starred: false, subject: "Fixture", timestamp: 1 }, text: "Body stays outside activity" } },
  ]);
  return { store, profile, intents: store.intents, activity: store.intents.activity! };
}

test("queued projections preserve current field ownership and cancellation never retires a started action", async () => {
  const { profile, intents, activity } = await setup();
  const first = await intents.register("m1", { starred: true, unread: false }, "tab");
  const second = await intents.register("m1", { starred: false }, "tab");
  const db = await openMailDatabase(profile);
  const project = () => queuedActionFields(db.transaction(["mailActions", "mailAliases", "mailMetadata", "mailIntents"], "readonly"));
  expect((await project()).get("m1")).toEqual({ starred: false, unread: false });
  await activity.cancelQueued((await activity.get(first.action!))!);
  expect((await project()).get("m1")).toEqual({ starred: false });
  const reviewed = (await activity.get(second.action!))!;
  await activity.update(second, { status: "Running" });
  await expect(activity.cancelQueued(reviewed)).rejects.toThrow("changed");
  expect((await activity.get(second.action!))!.status).toBe("Running");
  expect(await intents.effective(second)).toEqual({ starred: false });
  db.close();
});

test("saved Undo admits an inverse atomically, preserving newer fields and rejecting duplicate decisions", async () => {
  const { intents, activity } = await setup();
  const lease = await intents.register("m1", { starred: true, unread: false }, "tab");
  const source = (await activity.get(lease.action!))!.source;
  await activity.update(lease, { status: "Repair", receipt: { before: source, after: { ...source, starred: true, unread: false } }, applied: lease.fields });
  await intents.finish(lease, "applied");
  await activity.complete(lease, true);
  expect((await activity.page()).rows).toEqual([]);
  const saved = (await activity.page(undefined, true)).rows[0];
  await intents.register("m1", { unread: true }, "newer");
  const undo = await intents.registerUndo!(saved, "replacement");
  expect(undo.fields).toEqual({ starred: false });
  expect(await intents.effective(undo)).toEqual({ starred: false });
  expect((await activity.get(saved.id))!.undoneBy).toBe(undo.action);
  await expect(intents.registerUndo!(saved, "replacement")).rejects.toThrow("fresh review");
});

test("completed history is bounded independently of unresolved admission", async () => {
  const { intents, activity } = await setup();
  const unresolved = await intents.register("m1", { unread: false });
  await activity.update(unresolved, { status: "Uncertain" });
  for (let i = 0; i < 23; i++) {
    const lease = await intents.register("m1", { starred: !!(i % 2) });
    const source = (await activity.get(lease.action!))!.source;
    await activity.update(lease, { status: "Repair", receipt: { before: source, after: source } });
    await activity.complete(lease, true);
  }
  expect((await activity.page(undefined, true)).rows).toHaveLength(20);
  expect((await activity.page()).rows.map(row => row.id)).toEqual([unresolved.action]);
});

test("a reused cached identity cannot inherit a saved action projection", async () => {
  const { profile, store, intents } = await setup();
  await intents.register("m1", { folder: "Trash" }, "tab");
  const previous = await store.get<any>("mail", "m1");
  await store.commit([{ store: "mail", key: "m1", value: { ...previous, core: { ...previous.core, remote_id: "replacement" } } }]);
  const db = await openMailDatabase(profile);
  const projected = await queuedActionFields(db.transaction(["mailActions", "mailAliases", "mailMetadata", "mailIntents"], "readonly"));
  expect(projected.has("m1")).toBe(false);
  db.close();
});

test("cancellation cannot revive an older unresolved choice on the same field", async () => {
  const { intents, activity } = await setup();
  const first = await intents.register("m1", { starred: true });
  const second = await intents.register("m1", { starred: false });
  const row = (await activity.get(second.action!))!;
  await expect(activity.cancelQueued(row)).rejects.toThrow("earlier saved change");
  expect(await intents.effective(second)).toEqual({ starred: false });
  await activity.cancelQueued((await activity.get(first.action!))!);
  await activity.cancelQueued(row);
  expect((await activity.page()).rows).toEqual([]);
});

test("admission commits explicit intent and queued activity together, preserving old failures after restart", async () => {
  const { store, profile, intents, activity } = await setup();
  const first = await intents.register("m1", { starred: true });
  await activity.update(first, { status: "Rejected", error: "Fixture refusal" });
  const second = await intents.register("m1", { unread: false });
  await activity.complete(second);
  store.close();
  const reopened = await BrowserStore.open(profile); opened.push(reopened);
  const page = await reopened.intents.activity!.page();
  expect(page.rows).toHaveLength(1);
  expect(page.rows[0]).toMatchObject({ id: first.action, status: "Rejected", error: "Fixture refusal", lease: { fields: { starred: true } } });
  expect(JSON.stringify(page)).not.toContain("Body stays");
  expect(JSON.stringify(page)).not.toContain("Fixture\"");
});

test("failed admission rolls back both intent and activity", async () => {
  const { intents, activity, store } = await setup();
  const original = IDBObjectStore.prototype.put;
  vi.spyOn(IDBObjectStore.prototype, "put").mockImplementation(function (this: IDBObjectStore, ...args: Parameters<IDBObjectStore["put"]>) {
    if (this.name === "mailActions") throw Error("Fixture disk full");
    return original.apply(this, args);
  });
  await expect(intents.register("m1", { starred: true })).rejects.toThrow("disk full");
  expect(await store.get("mailIntents", "m1")).toBeUndefined();
  expect((await activity.page()).rows).toEqual([]);
});

test("dispatch and acknowledged receipt survive restart, reject replay and cannot become rejection", async () => {
  const { profile, store, intents, activity } = await setup();
  const lease = await intents.register("m1", { folder: "Archive" });
  await activity.update(lease, { status: "Running" });
  await expect(activity.update(lease, { status: "Running" })).rejects.toThrow("already started");
  const before = (await activity.get(lease.action!))!.source;
  const receipt = { before, after: { ...before, folder: "Archive", remoteId: "19" } };
  await activity.update(lease, { status: "Repair", receipt });
  await activity.update(lease, { status: "Rejected", error: "Late failure" });
  store.close();
  const reopened = await BrowserStore.open(profile); opened.push(reopened);
  expect(await reopened.intents.activity!.get(lease.action!)).toMatchObject({ status: "Repair", receipt });
  await expect(reopened.intents.activity!.dismissRejected(lease.action!)).rejects.toThrow("original recovery");
  await reopened.intents.activity!.complete(lease);
  await expect(reopened.intents.activity!.update(lease, { status: "Running" })).rejects.toThrow("already finished");
});

test("group claims do not create a second individual dispatcher record", async () => {
  const { intents, activity } = await setup();
  const revision = await intents.reserve();
  const lease = await intents.claim("m1", revision, { starred: true });
  await activity.update(lease, { status: "Running" });
  expect((await activity.page()).rows).toEqual([]);
});

test("bounded pages retain older errors and reject admission at capacity without losing intent", async () => {
  const { intents, activity, store } = await setup();
  for (let i = 0; i < 128; i++) await intents.register("m1", { starred: !!(i % 2) });
  const first = await activity.page(), second = await activity.page(first.next);
  expect(first.rows).toHaveLength(20);
  expect(second.rows).toHaveLength(20);
  expect(second.rows[0].id).toBe(actionId(21));
  const saved = await store.get("mailIntents", "m1");
  await expect(intents.register("m1", { unread: false })).rejects.toThrow("Activity");
  expect(await store.get("mailIntents", "m1")).toEqual(saved);
});

test("account removal reviews action receipts and deletes them atomically, fencing late updates", async () => {
  const { store, intents, activity } = await setup();
  const lease = await intents.register("m1", { starred: true });
  await intents.finish(lease, "failed");
  await activity.update(lease, { status: "Uncertain" });
  const review = removalPreview(await store.snapshot(reviewStores), "work");
  expect(review.changes).toBe(1);
  await expect(store.removeAccount(review, false)).rejects.toThrow();
  await store.removeAccount(review, true);
  expect((await activity.page()).rows).toEqual([]);
  await activity.update(lease, { status: "Repair" });
  expect((await activity.page()).rows).toEqual([]);
});

test("schema upgrade preserves source clock and fences older connections", async () => {
  const { profile, store } = await setup();
  const clock = await store.get("cacheState", "mail");
  const db = await openMailDatabase(profile);
  expect(db.version).toBe(18);
  expect(db.objectStoreNames.contains("mailActions")).toBe(true);
  expect(db.objectStoreNames.contains("accountConnections")).toBe(true);
  db.close();
  expect(await store.get("cacheState", "mail")).toEqual(clock);
});

test("resume adoption requires its captured owner and waits behind unresolved account work", async () => {
  const { intents, activity } = await setup();
  const first = await intents.register("m1", { starred: true }, "old-tab");
  const second = await intents.register("m1", { unread: false }, "old-tab");
  expect(await activity.adopt(second.action!, "old-tab", "new-tab")).toBeUndefined();
  expect(await activity.adopt(first.action!, "wrong-tab", "new-tab")).toBeUndefined();
  expect((await activity.adopt(first.action!, "old-tab", "new-tab"))?.owner).toBe("new-tab");
  await activity.update(first, { status: "Running" });
  expect(await activity.adopt(first.action!, "new-tab", "another-tab")).toBeUndefined();
  await activity.update(first, { status: "Uncertain" });
  expect(await activity.adopt(second.action!, "old-tab", "new-tab")).toBeUndefined();
  await activity.acceptReviewed((await activity.get(first.action!))!);
  expect((await activity.adopt(second.action!, "old-tab", "new-tab"))?.id).toBe(second.action);
});

test("checked uncertain acceptance retires only its own field intent and rejects a changed receipt", async () => {
  const { intents, activity, store } = await setup();
  const old = await intents.register("m1", { starred: true }, "old-tab");
  await activity.update(old, { status: "Uncertain" });
  const review = (await activity.get(old.action!))!;
  const newer = await intents.register("m1", { starred: false }, "new-tab");
  await activity.acceptReviewed(review);
  const intent = await store.get<any>("mailIntents", "m1");
  expect(intent.fields.starred).toMatchObject({ revision: newer.revision, value: false, status: "pending" });
  const changing = (await activity.get(newer.action!))!;
  await activity.update(newer, { status: "Repair", receipt: { before: changing.source, after: changing.source } });
  await expect(activity.acceptReviewed(changing)).rejects.toThrow("changed");
  expect((await activity.get(newer.action!))?.status).toBe("Repair");
});
