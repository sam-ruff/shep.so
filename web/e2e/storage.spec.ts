import { test, expect } from "@playwright/test";

test("IndexedDB upgrade preserves mail and seeds Sent roles; failed writes roll back all stores", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const evidence = await page.evaluate(async () => {
    // Separate synthetic profile, created before production BrowserStore opens.
    // This is a storage contract, separate from real-control mailbox scenarios.
    const user = "M".repeat(43),
      name = `shep.mail.v1.${user}`;
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open(name, 2);
      request.onupgradeneeded = () => {
        for (const store of [
          "accounts",
          "mail",
          "raw",
          "drafts",
          "outgoing",
          "draftFiles",
        ])
          request.result.createObjectStore(store);
      };
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(["mail", "raw", "outgoing"], "readwrite");
      tx.objectStore("mail").put(
        { id: "original", subject: "Storage fixture" },
        "original",
      );
      tx.objectStore("raw").put("Exact original fixture MIME", "original");
      for (const [draftId, folder] of [
        ["first", "Sent Mail"],
        ["second", "Old Sent"],
      ])
        tx.objectStore("outgoing").put(
          {
            id: "same-fixture-id",
            account: { id: "fixture" },
            draft: { id: draftId },
            sent: { state: "saved", receipt: { folder, remote_id: null } },
            wire: { raw: "unchanged MIME" },
          },
          draftId,
        );
      tx.oncomplete = () => resolve();
      tx.onabort = () => reject(tx.error);
    });
    db.close();
    const modulePath = "/src/storage.ts";
    const { BrowserStore } = await import(modulePath);
    const store = await BrowserStore.open(user);
    const migrated = await store.snapshot([
      "mail",
      "raw",
      "mailAliases",
      "mailRoles",
    ]);
    const duplicates = await store.submissions("same-fixture-id");
    let refused = false;
    try {
      await store.commit([
        { store: "mail", key: "partial", value: { id: "partial" } },
        {
          store: "mailAliases",
          key: "partial-alias",
          value: { target: () => "not cloneable" },
        },
      ]);
    } catch {
      refused = true;
    }
    const partial = await store.get("mail", "partial"),
      alias = await store.get("mailAliases", "partial-alias");
    await store.commit([
      { store: "mail", key: "valid", value: { id: "valid" } },
      {
        store: "mailAliases",
        key: "old",
        value: { alias: "old", target: "valid" },
      },
    ]);
    const pair = await store.snapshot(["mail", "mailAliases"]);
    const version = await new Promise<number>((resolve, reject) => {
      const r = indexedDB.open(name);
      r.onsuccess = () => {
        resolve(r.result.version);
        r.result.close();
      };
      r.onerror = () => reject(r.error);
    });
    return {
      migrated,
      duplicates,
      refused,
      partial: partial ?? null,
      alias: alias ?? null,
      pair,
      version,
    };
  });
  expect(evidence.version).toBe(10);
  expect(evidence.migrated.mail).toEqual([
    { id: "original", subject: "Storage fixture" },
  ]);
  expect(evidence.migrated.raw).toEqual(["Exact original fixture MIME"]);
  expect(evidence.migrated.mailAliases).toEqual([]);
  expect(evidence.migrated.mailRoles).toEqual([
    { account: "fixture", acknowledged: ["Sent Mail", "Old Sent"] },
  ]);
  expect(evidence.duplicates).toHaveLength(2);
  expect(
    evidence.duplicates.every(
      (r: { wire: { raw: string } }) => r.wire.raw === "unchanged MIME",
    ),
  ).toBe(true);
  expect(evidence.refused).toBe(true);
  expect(evidence.partial).toBeNull();
  expect(evidence.alias).toBeNull();
  expect(evidence.pair.mailAliases).toEqual([
    { alias: "old", target: "valid" },
  ]);
  expect(evidence.pair.mail.some((m: { id: string }) => m.id === "valid")).toBe(
    true,
  );
});

test("account removal rechecks its transaction and refuses late tab writes without retaining deleted content", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const storeModule = "/src/storage.ts",
      removalModule = "/src/account_removal.ts",
      providerModule = "/src/provider.ts";
    const { BrowserStore } = await import(storeModule);
    const { removalPreview, reviewStores } = await import(removalModule);
    const { GatewayRepository } = await import(providerModule);
    const store = await BrowserStore.open("R".repeat(43));
    const account = { id: "fixture", email: "owner@example.test" };
    const draft = {
      id: "draft",
      accountId: account.id,
      body: "PRIVATE REMOVED TEXT",
      revision: 1,
    };
    const mail = {
      core: { id: "mail", account_id: account.id },
      text: "PRIVATE REMOVED TEXT",
    };
    await store.commit([
      { store: "accounts", key: account.id, value: account },
      { store: "accounts", key: "other", value: { id: "other" } },
      { store: "mail", key: "mail", value: mail },
      { store: "raw", key: "mail", value: "PRIVATE RAW" },
      { store: "drafts", key: "draft", value: draft },
      {
        store: "draftFiles",
        key: "file",
        value: {
          draftId: "draft",
          info: { id: "file", name: "private.bin" },
          blob: new Blob(["PRIVATE BYTES"]),
        },
      },
      {
        store: "mailAliases",
        key: "old",
        value: { alias: "old", target: "mail" },
      },
    ]);
    const old = removalPreview(await store.snapshot(reviewStores), account.id);
    await store.commit([
      { store: "drafts", key: "draft", value: { ...draft, revision: 2 } },
    ]);
    let stale = false;
    try {
      await store.removeAccount(old, false);
    } catch (e) {
      stale = String(e).includes("Local data changed");
    }
    const retained = !!(await store.get("accounts", account.id));
    const current = removalPreview(
      await store.snapshot(reviewStores),
      account.id,
    );
    let release!: () => void, entered!: () => void;
    const acquired = new Promise<void>((r) => (entered = r)),
      gate = new Promise<void>((r) => (release = r));
    const held = navigator.locks.request(
      `shep.${"R".repeat(43)}.account.fixture`,
      async () => {
        entered();
        await gate;
      },
    );
    await acquired;
    const guarded = new GatewayRepository(
      { user_id: "R".repeat(43), csrf: "C".repeat(43) },
      store,
    );
    let occupied = false;
    try {
      await guarded.removeAccount(
        await guarded.removalPreview(account.id),
        false,
      );
    } catch (e) {
      occupied = String(e).includes("operation in progress");
    }
    release();
    await held;
    if (!occupied) throw Error("Removal did not refuse an occupied account");
    await store.removeAccount(current, false);
    await store.removeAccount(current, false);
    let late = false;
    try {
      await store.commit([
        {
          store: "accounts",
          key: "other",
          value: { id: "other", name: "must roll back" },
        },
        { store: "drafts", key: "draft", value: { ...draft, revision: 999 } },
      ]);
    } catch (e) {
      late = String(e).includes("removed");
    }
    const other = await store.get("accounts", "other");
    const data = await store.snapshot(reviewStores);
    let requests = 0;
    const repo = new GatewayRepository(
      { user_id: "R".repeat(43), csrf: "C".repeat(43) },
      store,
      async () => {
        requests++;
        throw Error("Must not request");
      },
      async (_name: any, fn: any) => fn(),
    );
    let reconnect = false;
    try {
      await repo.connect(account, "new", "new");
    } catch (e) {
      reconnect = String(e).includes("removed");
    }
    const raw = await store.get("raw", "mail");
    store.close();
    return { stale, retained, late, other, data, raw, reconnect, requests };
  });
  expect(result.stale).toBe(true);
  expect(result.retained).toBe(true);
  expect(result.late).toBe(true);
  expect(result.other).toEqual({ id: "other" });
  expect(result.requests).toBe(0);
  expect(result.reconnect).toBe(true);
  expect(result.raw).toBeUndefined();
  for (const name of [
    "mail",
    "drafts",
    "draftFiles",
    "outgoing",
    "mailAliases",
  ])
    expect(result.data[name]).toEqual([]);
  expect(JSON.stringify(result.data)).not.toContain("PRIVATE");
});
