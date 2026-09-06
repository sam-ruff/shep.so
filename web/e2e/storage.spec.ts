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
  expect(evidence.version).toBe(3);
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
