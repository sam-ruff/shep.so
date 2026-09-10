import { test, expect } from "@playwright/test";

async function setup(page: import("@playwright/test").Page, count = 125) {
  await page.goto("/preview.html");
  await page.evaluate(async (count) => {
    const storagePath = "/src/storage.ts",
      workerPath = "/src/mailbox_worker_client.ts";
    const { BrowserStore, openMailDatabase } = await import(storagePath);
    const { MailboxWorkerClient } = await import(workerPath);
    const user = "Q".repeat(43),
      store = await BrowserStore.open(user);
    const mail = (i: number) => ({
      core: {
        id: `m${String(i).padStart(6, "0")}`,
        account_id: "work",
        remote_id: String(i),
        folder: "INBOX",
        sender: '"Sender" <sender@example.test>',
        recipient: "work@example.test",
        subject: `Message ${i}`,
        preview: "Synthetic mailbox",
        timestamp: 1000000 - i,
        unread: i % 2 === 0,
        starred: i % 3 === 0,
        attachment_count: 0,
      },
      text: `Exact body needle ${i}. CAFÉ 😀😁😂 quoted "words" 50%_a`,
    });
    await store.commit([
      {
        store: "accounts",
        key: "work",
        value: {
          id: "work",
          email: "work@example.test",
          sent_folder: "Sent Mail",
        },
      },
    ]);
    for (let i = 0; i < count; i += 500)
      await store.commit(
        Array.from({ length: Math.min(count - i, 500) }, (_, n) => {
          const value = mail(i + n);
          return { store: "mail", key: value.core.id, value };
        }),
      );
    Object.assign(window, {
      mailboxTest: {
        user,
        store,
        mail,
        BrowserStore,
        MailboxWorkerClient,
        openMailDatabase,
        worker: new MailboxWorkerClient(user),
      },
    });
  }, count);
}

test("persistent mailbox returns bounded metadata, exact search, projected scope and separate bodies", async ({
  page,
}) => {
  await setup(page);
  const result = await page.evaluate(async () => {
    const { worker, store, mail } = (window as any).mailboxTest;
    const query = (scope: any, offset = 0, observed: string[] = []) =>
      worker.page({ scope: { folder: "Inbox", ...scope }, offset, observed });
    const first = await query({}),
      second = await query({}, 50),
      last = await query({}, 100);
    const search = await query({ query: 'NEEDLE café 😀😁😂 "words" 50%_a' });
    const short = await query({ query: "é" }),
      absent = await query({ query: "uncached" });
    const oldest = await query({ oldest: true });
    const projected = await query({
      filter: "Unread",
      projection: { m000000: { folder: "Archive" }, m000001: { unread: true } },
    });
    const archive = await query({
      folder: "Archive",
      projection: { m000000: { folder: "Archive" } },
    });
    const detail = await worker.detail("m000000");
    const changed = mail(0);
    changed.core.folder = "Sent Mail";
    changed.core.id = "adopted";
    changed.text = "Updated body";
    await store.commit([
      { store: "mail", key: "m000000" },
      { store: "mail", key: "adopted", value: changed },
      {
        store: "mailAliases",
        key: "m000000",
        value: { alias: "m000000", target: "adopted" },
      },
    ]);
    const alias = await query(
      { folder: "Sent", projection: { m000000: { starred: true } } },
      0,
      ["m000000"],
    );
    const changedDetail = await worker.detail("m000000");
    // Folder roles change without a mail revision and still affect the query.
    await store.commit([
      {
        store: "mailRoles",
        key: "work",
        value: { account: "work", acknowledged: ["INBOX"] },
      },
    ]);
    const roleChanged = await query({ folder: "Sent" });
    await store.commit([
      { store: "mail", key: "damaged", value: { ...mail(126), text: null } },
    ]);
    let damagedBody = "";
    try {
      await worker.detail("damaged");
    } catch (error) {
      damagedBody = String(error);
    }
    await worker.close();
    store.close();
    return {
      first,
      second,
      last,
      search,
      short,
      absent,
      oldest,
      projected,
      archive,
      detail,
      alias,
      changedDetail,
      roleChanged,
      damagedBody,
    };
  });
  expect(result.first.total).toBe(125);
  expect(result.first.unread).toBe(63);
  expect(result.first.rows).toHaveLength(50);
  expect(
    result.first.rows.every(
      (m: any) =>
        m.body === "" &&
        m.bodyLoaded === false &&
        !("text" in m) &&
        !("raw" in m),
    ),
  ).toBe(true);
  expect(result.second.rows[0].id).toBe("m000050");
  expect(result.last.rows).toHaveLength(25);
  expect(result.search.total).toBe(125);
  expect(result.short.total).toBe(125);
  expect(result.absent.total).toBe(0);
  expect(result.oldest.rows[0].id).toBe("m000124");
  expect(result.projected.total).toBe(63);
  expect(result.projected.unread).toBe(63);
  expect(result.projected.rows[0].id).toBe("m000001");
  expect(result.archive.total).toBe(1);
  expect(result.detail.body).toContain("Exact body needle 0");
  expect(result.alias.total).toBe(1);
  expect(result.alias.rows[0].id).toBe("adopted");
  expect(result.alias.rows[0].starred).toBe(true);
  expect(result.alias.aliases).toEqual({ m000000: "adopted" });
  expect(result.changedDetail.body).toBe("Updated body");
  expect(result.roleChanged.total).toBe(125);
  expect(result.damagedBody).toContain("cached message body is damaged");
});

test("mailbox index survives workers, cooperates across tabs and rejects a recreated source with the same revision", async ({
  page,
  context,
}) => {
  await setup(page);
  const first = await page.evaluate(async () =>
    (window as any).mailboxTest.worker.page({
      scope: { folder: "Inbox" },
      offset: 0,
    }),
  );
  const other = await context.newPage();
  await other.goto("/preview.html");
  const simultaneous = await Promise.all([
    page.evaluate(async () =>
      (window as any).mailboxTest.worker.page({
        scope: { folder: "Inbox" },
        offset: 100,
      }),
    ),
    other.evaluate(async () => {
      const path = "/src/mailbox_worker_client.ts",
        { MailboxWorkerClient } = await import(path);
      const worker = new MailboxWorkerClient("Q".repeat(43));
      const result = await worker.page({
        scope: { folder: "Inbox" },
        offset: 50,
      });
      await worker.close();
      return result;
    }),
  ]);
  expect(simultaneous[0].rows).toHaveLength(25);
  expect(simultaneous[1].rows[0].id).toBe("m000050");
  const recreated = await page.evaluate(async (revision) => {
    const t = (window as any).mailboxTest;
    await t.worker.close();
    t.store.close();
    await new Promise<void>((resolve, reject) => {
      const request = indexedDB.deleteDatabase(`shep.mail.v1.${t.user}`);
      request.onsuccess = () => resolve();
      request.onerror = () => reject(request.error);
    });
    const store = await t.BrowserStore.open(t.user);
    const value = t.mail(900);
    await store.commit([
      {
        store: "accounts",
        key: "work",
        value: { id: "work", email: "replacement@example.test" },
      },
      { store: "mail", key: value.core.id, value },
    ]);
    // Same revision number is legal in a new source incarnation.
    const state = await store.get("cacheState", "mail");
    await store.commit([
      {
        store: "cacheState",
        key: "mail",
        value: { ...state, revision, floor: revision },
      },
    ]);
    const worker = new t.MailboxWorkerClient(t.user);
    const result = await worker.page({ scope: { folder: "Inbox" }, offset: 0 });
    await worker.close();
    store.close();
    return result;
  }, first.revision);
  expect(recreated.epoch).not.toBe(first.epoch);
  expect(recreated.revision).toBe(first.revision);
  expect(recreated.total).toBe(1);
  expect(recreated.rows[0].id).toBe("m000900");
  expect(recreated.rows[0].account).toBe("replacement@example.test");
});

test("100000-message index rebuild leaves draft writes independent and replays overflow and removals", async ({
  page,
}) => {
  test.setTimeout(180000);
  page.on("console", (message) => {
    if (message.text().startsWith("mailbox-stage")) console.log(message.text());
  });
  await setup(page, 100000);
  console.log("mailbox-stage seeded");
  const result = await page.evaluate(async () => {
    const t = (window as any).mailboxTest;
    let finished = false;
    const started = new Promise<void>((resolve) =>
      t.worker.addEventListener("activity", () => resolve(), { once: true }),
    );
    const building = t.worker
      .page({ scope: { folder: "Inbox" }, offset: 99950 })
      .then((value: any) => {
        finished = true;
        return value;
      });
    await started;
    console.log("mailbox-stage snapshot");
    // A disjoint readonly mailbox snapshot must not own the mail write lock.
    await t.store.commit([
      {
        store: "drafts",
        key: "draft",
        value: {
          id: "draft",
          body: "Typing while indexing",
          accountId: "work",
        },
      },
    ]);
    const draftBeforeCompletion = !finished;
    console.log("mailbox-stage draft saved");
    const first = await building;
    console.log("mailbox-stage first index complete");
    const db = await t.openMailDatabase(t.user);
    // Fail an incremental update; no partial checkpoint may be committed.
    await t.store.commit([
      {
        store: "mail",
        key: "bad",
        value: { core: { ...t.mail(1).core, id: "bad" }, text: null },
      },
    ]);
    let failed = false;
    try {
      await t.worker.page({ scope: { folder: "Inbox" }, offset: 0 });
    } catch {
      failed = true;
    }
    console.log("mailbox-stage failed update rolled back");
    await t.store.commit([{ store: "mail", key: "bad" }]);
    const changes = Array.from({ length: 1100 }, (_, i) => ({
      store: "mail",
      key: t.mail(i).core.id,
    }));
    await t.store.commit(changes);
    const overflow = await t.worker.page({
      scope: { folder: "Inbox", query: "needle" },
      offset: 0,
    });
    console.log("mailbox-stage overflow complete");
    await t.store.commit([{ store: "accounts", key: "work" }]);
    const removed = await t.worker.page({
      scope: { folder: "Inbox" },
      offset: 0,
    });
    let detailRejected = false;
    try {
      await t.worker.detail("m099999");
    } catch {
      detailRejected = true;
    }
    await t.worker.close();
    t.store.close();
    db.close();
    return {
      draftBeforeCompletion,
      first,
      failed,
      overflow,
      removed,
      detailRejected,
    };
  });
  expect(result.draftBeforeCompletion).toBe(true);
  expect(result.first.total).toBe(100000);
  expect(result.first.rows).toHaveLength(50);
  expect(result.first.rows[0].id).toBe("m099950");
  expect(result.failed).toBe(true);
  expect(result.overflow.total).toBe(98900);
  expect(result.overflow.rows[0].id).toBe("m001100");
  expect(result.removed.total).toBe(0);
  expect(result.removed.unread).toBe(0);
  expect(result.detailRejected).toBe(true);
});

test("mailbox queue is bounded and abrupt worker loss releases its index for recovery", async ({
  page,
}) => {
  await setup(page, 5000);
  const result = await page.evaluate(async () => {
    const t = (window as any).mailboxTest;
    const query = { scope: { folder: "Inbox" }, offset: 0 };
    await t.worker.page(query);
    let release!: () => void;
    let acquired!: () => void;
    const hasLock = new Promise<void>((resolve) => {
      acquired = resolve;
    });
    const held = navigator.locks.request(
      `shep.${t.user}.mailbox-index`,
      async () => {
        acquired();
        await new Promise<void>((resolve) => {
          release = resolve;
        });
      },
    );
    await hasLock;
    const queued = Array.from({ length: 50 }, () =>
      t.worker.page(query).then(
        () => true,
        () => false,
      ),
    );
    const rejectedWhileHeld = await Promise.race(queued);
    release();
    await held;
    const settled = await Promise.all(queued);
    // The next snapshot is deliberately interrupted before its rebuild finishes.
    const state = await t.store.get("cacheState", "mail");
    await t.store.commit([
      {
        store: "cacheState",
        key: "mail",
        value: {
          ...state,
          floor: state.revision + 1,
          revision: state.revision + 1,
        },
      },
    ]);
    const active = new Promise<void>((resolve) =>
      t.worker.addEventListener("activity", () => resolve(), { once: true }),
    );
    const interrupted = t.worker.page(query).then(
      () => false,
      () => true,
    );
    await active;
    t.worker.terminate();
    const lost = await interrupted;
    const replacement = new t.MailboxWorkerClient(t.user);
    const recovered = await replacement.page(query);
    await replacement.close();
    t.store.close();
    return { rejectedWhileHeld, settled, lost, recovered };
  });
  expect(result.rejectedWhileHeld).toBe(false);
  expect(result.settled.filter(Boolean)).toHaveLength(32);
  expect(result.lost).toBe(true);
  expect(result.recovered.total).toBe(5000);
  expect(result.recovered.rows).toHaveLength(50);
});

test("indexed search preserves the list predicate for punctuation, combining marks and embedded NUL", async ({
  page,
}) => {
  await setup(page, 0);
  const result = await page.evaluate(async () => {
    const t = (window as any).mailboxTest,
      path = "/src/mail_query.ts",
      { mailMatches, senderName } = await import(path);
    const texts = [
      "before\0after abracadabra",
      "é e\u0301\u0327 capital İ ſ 𐐀",
      'OR AND NEAR (phrase) "quote" %_*,. 😀😁😂',
      "unrelated",
    ];
    const values = texts.map((text, i) => ({ ...t.mail(i), text }));
    await t.store.commit(
      values.map((value: any) => ({
        store: "mail",
        key: value.core.id,
        value,
      })),
    );
    const queries = [
      "after",
      "abracadabra",
      "before\0after",
      "e\u0301\u0327",
      "İ",
      "ſ",
      "𐐀",
      "OR",
      "NEAR",
      '"quote"',
      "%_*,.",
      "😀😁😂",
      "unrelated",
      "OR 😀😁😂",
      "no match",
    ];
    const results = [];
    for (const query of queries) {
      const scope = { folder: "Inbox", query };
      const expected = values
        .filter((v: any) =>
          mailMatches(
            { ...v.core, sender: senderName(v.core.sender), body: v.text },
            scope,
          ),
        )
        .map((v: any) => v.core.id);
      const found = await t.worker.page({ scope, offset: 0 });
      results.push({
        query,
        expected,
        actual: found.rows.map((m: any) => m.id),
      });
    }
    await t.worker.close();
    t.store.close();
    return results;
  });
  for (const value of result)
    expect(value.actual, value.query).toEqual(value.expected);
});

for (const version of [8, 9])
  test(`mail schema ${version} upgrades without losing cache clocks, incarnation, indexed records, intents or Sent roles`, async ({
    page,
  }) => {
    await page.goto("/preview.html");
    const result = await page.evaluate(async (version) => {
      const user = "V".repeat(43),
        path = "/src/storage.ts",
        { stores, BrowserStore } = await import(path);
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        const r = indexedDB.open(`shep.mail.v1.${user}`, version);
        r.onupgradeneeded = () => {
          for (const name of stores) r.result.createObjectStore(name);
          r.transaction!.objectStore("mailMetadata").createIndex(
            "newest",
            "newest",
          );
          r.transaction!.objectStore("mailMetadata").createIndex(
            "oldest",
            "oldest",
          );
          r.transaction!.objectStore("outgoing").createIndex(
            "submission",
            "id",
          );
        };
        r.onerror = () => reject(r.error);
        r.onsuccess = () => resolve(r.result);
      });
      await new Promise<void>((resolve, reject) => {
        const tx = db.transaction(
          [
            "cacheState",
            "intentState",
            "mailIntents",
            "mailRoles",
            "mail",
            "accounts",
          ],
          "readwrite",
        );
        tx.objectStore("cacheState").put(
          {
            revision: 456,
            floor: 123,
            ...(version === 9 ? { epoch: "existing-incarnation" } : {}),
          },
          "mail",
        );
        tx.objectStore("accounts").put(
          { id: "work", email: "work@example.test" },
          "work",
        );
        tx.objectStore("mail").put(
          {
            core: { id: "physical", account_id: "work", folder: "INBOX" },
            text: "Preserved cached body",
          },
          "legacy",
        );
        tx.objectStore("intentState").put({ revision: 789 }, "clock");
        tx.objectStore("mailIntents").put(
          { id: "message", applied: { folder: 788 } },
          "message",
        );
        tx.objectStore("mailRoles").put(
          { account: "work", acknowledged: ["Recent Sent"] },
          "work",
        );
        tx.oncomplete = () => resolve();
        tx.onabort = () => reject(tx.error);
      });
      let closed = false;
      db.onversionchange = () => {
        closed = true;
        db.close();
      };
      const store = await BrowserStore.open(user);
      const first = await store.get("cacheState", "mail"),
        preserved = await store.snapshot([
          "intentState",
          "mailIntents",
          "mailRoles",
        ]);
      store.close();
      const reopened = await BrowserStore.open(user),
        second = await reopened.get("cacheState", "mail");
      reopened.close();
      const workerPath = "/src/mailbox_worker_client.ts";
      const { MailboxWorkerClient } = await import(workerPath),
        worker = new MailboxWorkerClient(user);
      const scan = await worker.scan({ account: "work", serverId: "physical" }),
        detail = await worker.detail("legacy");
      await worker.close();
      return { first, second, preserved, closed, scan, detail };
    }, version);
    expect(result.closed).toBe(true);
    expect(result.first).toMatchObject({ revision: 456, floor: 123 });
    if (version === 9) expect(result.first.epoch).toBe("existing-incarnation");
    else expect(result.first.epoch).toMatch(/^[\w-]{36}$/);
    expect(result.scan.rows.map((r: any) => r.key)).toEqual(["legacy"]);
    expect(result.detail.body).toBe("Preserved cached body");
    expect(result.second).toEqual(result.first);
    expect(result.preserved).toEqual({
      intentState: [{ revision: 789 }],
      mailIntents: [{ id: "message", applied: { folder: 788 } }],
      mailRoles: [{ account: "work", acknowledged: ["Recent Sent"] }],
    });
  });

test("independent body and metadata reads finish while indexing waits, scans are bounded and shutdown stays closed", async ({
  page,
}) => {
  await setup(page);
  const result = await page.evaluate(async () => {
    const t = (window as any).mailboxTest;
    let release!: () => void, acquired!: () => void;
    const ready = new Promise<void>((r) => (acquired = r));
    const held = navigator.locks.request(
      `shep.${t.user}.mailbox-index`,
      async () => {
        acquired();
        await new Promise<void>((r) => (release = r));
      },
    );
    await ready;
    let completed = false;
    const pending = t.worker
      .page({ scope: { folder: "Inbox" }, offset: 0 })
      .then((p: any) => {
        completed = true;
        return p;
      });
    const metadata = await t.worker.metadata("m000124");
    const body = await t.worker.detail("m000124");
    const scans = [];
    let after: string | null = null;
    do {
      const scan = await t.worker.scan({
        account: "work",
        folder: "INBOX",
        after,
      });
      scans.push(scan);
      after = scan.next;
    } while (after);
    const identity = await t.worker.scan({
      account: "work",
      serverId: "m000124",
    });
    const unrelated = await t.worker.scan({ account: "absent" });
    const beforeRelease = completed;
    release();
    await held;
    await pending;
    await t.worker.close();
    const closed = await Promise.allSettled([
      t.worker.detail("m000124"),
      t.worker.metadata("m000124"),
      t.worker.prefetch("m000124"),
      t.worker.scan({ account: "work" }),
    ]);
    t.store.close();
    return {
      metadata,
      body,
      scans,
      identity,
      unrelated,
      beforeRelease,
      closed: closed.map((x) => x.status),
    };
  });
  expect(result.beforeRelease).toBe(false);
  expect(result.metadata.mail.subject).toBe("Message 124");
  expect(result.metadata.mail.bodyLoaded).toBe(false);
  expect(result.body.body).toContain("Exact body needle 124");
  expect(result.scans.map((p: any) => p.rows.length)).toEqual([50, 50, 25]);
  const rows = result.scans.flatMap((p: any) => p.rows);
  expect(new Set(rows.map((r: any) => r.key)).size).toBe(125);
  expect(
    rows.every((r: any) => !("text" in r) && !("raw" in r) && !("reply" in r)),
  ).toBe(true);
  expect(result.identity.rows.map((r: any) => r.key)).toEqual(["m000124"]);
  expect(result.unrelated.rows).toEqual([]);
  expect(result.closed).toEqual([
    "rejected",
    "rejected",
    "rejected",
    "rejected",
  ]);
});
