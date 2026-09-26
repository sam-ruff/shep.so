import "fake-indexeddb/auto";
import { afterEach, expect, test, vi } from "vitest";
import { BrowserStore, type LocalStore } from "./storage";
import { resolveMail } from "./sent_cache";
import type { RecordMail } from "./provider";

const opened: BrowserStore[] = [];
afterEach(() => {
  for (const store of opened.splice(0)) store.close();
  vi.restoreAllMocks();
});
const mail = (id: string) => ({
  core: {
    id,
    account_id: "work",
    remote_id: id,
    folder: "INBOX",
    unread: true,
    starred: false,
    subject: `Fixture ${id}`,
    timestamp: 1,
  },
  text: "Fixture body",
});
async function setup() {
  const store = await BrowserStore.open("R".repeat(43));
  opened.push(store);
  await store.commit([
    {
      store: "accounts",
      key: "work",
      value: { id: "work", email: "work@example.test" },
    },
    { store: "mail", key: "m2", value: mail("m2") },
    {
      store: "mailAliases",
      key: "m1",
      value: { alias: "m1", target: "m2" },
    },
  ]);
  return store;
}

test("the browser store resolves an alias and its message in one read", async () => {
  const store = await setup();
  const transaction = vi.spyOn(IDBDatabase.prototype, "transaction");
  expect((await resolveMail(store, "m1"))?.core.id).toBe("m2");
  expect(transaction).toHaveBeenCalledTimes(1);
  expect((await resolveMail(store, "m2"))?.core.id).toBe("m2");
  expect(await resolveMail(store, "missing")).toBeUndefined();
});

test("a store without the combined read follows the alias with separate reads", async () => {
  const store = await setup();
  const plain: LocalStore = {
    all: (name, limit) => store.all(name, limit),
    get: (name, key) => store.get(name, key),
    commit: (changes) => store.commit(changes),
    snapshot: (names) => store.snapshot(names),
    submissions: (id) => store.submissions(id),
  };
  const found: RecordMail | undefined = await resolveMail(plain, "m1");
  expect(found?.core.id).toBe("m2");
  expect(await resolveMail(plain, "missing")).toBeUndefined();
});
