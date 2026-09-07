import { test, expect } from "@playwright/test";

test("owned browser selection spans pages, freezes exact membership and follows current metadata", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const storagePath = "/src/storage.ts",
      workerPath = "/src/selection_worker_client.ts";
    const { BrowserStore } = await import(storagePath);
    const { SelectionWorkerClient } = await import(workerPath);
    const user = "S".repeat(43),
      store = await BrowserStore.open(user);
    const account = {
      id: "work",
      email: "work@example.test",
      sent_folder: "Sent Mail",
    };
    const mail = (i: number) => ({
      core: {
        id: `m${String(i).padStart(3, "0")}`,
        account_id: "work",
        remote_id: String(i),
        folder: "INBOX",
        sender: "Sender <sender@example.test>",
        recipient: "work@example.test",
        subject: `Message ${i}`,
        preview: "Synthetic selection",
        timestamp: 10000 - i,
        unread: i % 2 === 0,
        starred: i % 3 === 0,
        attachment_count: 0,
      },
      text: `Cached body needle ${i}`,
    });
    await store.commit([
      { store: "accounts", key: "work", value: account },
      ...Array.from({ length: 125 }, (_, i) => ({
        store: "mail",
        key: mail(i).core.id,
        value: mail(i),
      })),
    ]);
    const worker = new SelectionWorkerClient(user);
    const scope = { folder: "Inbox" };
    const capture = (id: string, revision: number, all: boolean) => ({
      kind: "capture",
      id,
      revision,
      scope,
      all,
    });
    const change = (id: string, expected: number, value: unknown) => ({
      kind: "change",
      id,
      expected,
      scope,
      change: value,
    });
    const first = await worker.selection(capture("live", 0, true), [
      "m000",
      "m100",
      "m124",
    ]);
    const arrival = mail(125);
    arrival.core.timestamp = 10001;
    await store.commit([
      { store: "mail", key: arrival.core.id, value: arrival },
    ]);
    const passive = await worker.selection({ kind: "observe", id: "live" }, [
      "m125",
    ]);
    const explicit = await worker.selection(
      change("live", 0, {
        kind: "set",
        id: "m125",
        selected: true,
        clear_others: false,
      }),
      ["m125"],
    );
    const recapture = await worker.selection(capture("live", 2, true), [
      "m125",
      "m000",
      "m100",
    ]);
    const range = await worker.selection(
      change("live", 2, {
        kind: "range",
        anchor: "m000",
        target: "m100",
        additive: false,
      }),
    );
    const frozen = await worker.selection({
      kind: "freeze",
      id: "live",
      expected: 3,
      target: "review",
    });
    const cleared = await worker.selection(
      change("live", 3, { kind: "clear" }),
    );
    let frozenRefused = false,
      stale = false;
    try {
      await worker.selection(change("review", 0, { kind: "clear" }));
    } catch {
      frozenRefused = true;
    }
    try {
      await worker.selection(change("live", 3, { kind: "clear" }));
    } catch {
      stale = true;
    }
    // Identity handover plus flags are one ordinary production cache transaction.
    const adopted = mail(0);
    adopted.core.id = "adopted";
    adopted.core.folder = "Archive";
    adopted.core.unread = false;
    await store.commit([
      { store: "mail", key: "m000" },
      { store: "mail", key: "adopted", value: adopted },
      {
        store: "mailAliases",
        key: "m000",
        value: { alias: "m000", target: "adopted" },
      },
    ]);
    const alias = await worker.selection({ kind: "observe", id: "review" }, [
      "m000",
    ]);
    await store.commit([{ store: "mail", key: "m001" }]);
    const missing = await worker.selection({ kind: "observe", id: "review" }, [
      "m001",
    ]);
    let after = null,
      total = 0,
      pages = 0,
      seen = new Set<string>();
    do {
      const chunk = await worker.selection({
        kind: "page",
        id: "review",
        expected: 0,
        after,
      });
      if (chunk.rows.length > 50) throw Error("Unbounded selection page");
      for (const row of chunk.rows) {
        if ("body" in row || "raw" in row || seen.has(row.id))
          throw Error("Invalid selection metadata page");
        seen.add(row.id);
      }
      total += chunk.rows.length;
      pages++;
      after = chunk.next_after;
    } while (after !== null);
    await worker.close();
    const reopened = new SelectionWorkerClient(user);
    let expired = false;
    try {
      await reopened.selection({ kind: "observe", id: "review" });
    } catch {
      expired = true;
    }
    await reopened.close();
    const unchanged = (await store.get("mail", "m002")).core;
    store.close();
    return {
      first,
      passive,
      explicit,
      recapture,
      range,
      frozen,
      cleared,
      frozenRefused,
      stale,
      alias,
      missing,
      total,
      pages,
      expired,
      unchanged,
    };
  });
  expect(result.first.selected).toBe(125);
  expect(result.first.positions.m100).toBe(100);
  expect(result.first.unread).toBe(63);
  expect(result.passive.selected).toBe(125);
  expect(result.passive.visible).toEqual([]);
  expect(result.explicit.selected).toBe(126);
  expect(result.recapture.positions.m125).toBe(0);
  expect(result.range.selected).toBe(101);
  expect(result.frozen.frozen).toBe(true);
  expect(result.cleared.selected).toBe(0);
  expect(result.frozenRefused && result.stale).toBe(true);
  expect(result.alias.visible).toEqual(["adopted"]);
  expect(result.alias.groups).toContainEqual({
    account: "work",
    folder: "Archive",
    total: 1,
    unread: 0,
    starred: 1,
  });
  expect(result.missing.selected).toBe(101);
  expect(result.missing.available).toBe(100);
  expect(result.total).toBe(100);
  expect(result.pages).toBe(3);
  expect(result.expired).toBe(true);
  expect(result.unchanged.unread).toBe(true);
});

test("version-four upgrade keeps newer Sent roles while adding selection indexes", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const user = "V".repeat(43),
      name = `shep.mail.v1.${user}`;
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const r = indexedDB.open(name, 4);
      r.onupgradeneeded = () => {
        for (const name of [
          "accounts",
          "mail",
          "raw",
          "drafts",
          "outgoing",
          "draftFiles",
          "mailAliases",
          "mailRoles",
          "removedAccounts",
        ])
          r.result.createObjectStore(name);
      };
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error);
    });
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(["outgoing", "mailRoles", "mail"], "readwrite");
      tx.objectStore("outgoing").put(
        {
          id: "original",
          account: { id: "work" },
          sent: { state: "saved", receipt: { folder: "Old Sent" } },
        },
        "draft",
      );
      tx.objectStore("mailRoles").put(
        {
          account: "work",
          discovered: "New Sent",
          acknowledged: ["Updated Sent"],
        },
        "work",
      );
      tx.objectStore("mail").put(
        {
          core: { id: "original", account_id: "work", timestamp: 42 },
          text: "Exact cached text",
        },
        "original",
      );
      tx.oncomplete = () => resolve();
      tx.onabort = () => reject(tx.error);
    });
    db.close();
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(user);
    const roles = await store.all("mailRoles"),
      mail = await store.get("mail", "original"),
      metadata = await store.get("mailMetadata", "original"),
      state = await store.get("cacheState", "mail");
    store.close();
    return { roles, mail, metadata, state };
  });
  expect(result.roles).toEqual([
    { account: "work", discovered: "New Sent", acknowledged: ["Updated Sent"] },
  ]);
  expect(result.mail.text).toBe("Exact cached text");
  expect(result.mail._shepNewest).toBeUndefined();
  expect(result.metadata.newest).toEqual([-42, "original"]);
  expect(result.state).toEqual({ revision: 0, floor: 0 });
});

test("worker-owned selections stay private and disappear with their worker", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const path = "/src/storage.ts",
      workerPath = "/src/selection_worker_client.ts";
    const { BrowserStore } = await import(path),
      { SelectionWorkerClient } = await import(workerPath);
    const user = "O".repeat(43),
      store = await BrowserStore.open(user);
    await store.commit([
      { store: "accounts", key: "work", value: { id: "work" } },
      {
        store: "mail",
        key: "message",
        value: {
          core: {
            id: "message",
            account_id: "work",
            folder: "INBOX",
            sender: "Sender",
            subject: "Worker fixture",
            timestamp: 1,
            unread: true,
            starred: false,
          },
          text: "Worker fixture",
        },
      },
    ]);
    const first = new SelectionWorkerClient(user),
      second = new SelectionWorkerClient(user);
    const capture = {
      kind: "capture",
      id: "same-token",
      revision: 0,
      scope: { folder: "Inbox" },
      all: true,
    };
    await first.selection(capture);
    await second.selection(capture);
    await second.selection({
      kind: "change",
      id: "same-token",
      expected: 0,
      scope: { folder: "Inbox" },
      change: { kind: "clear" },
    });
    const retained = await first.selection({
      kind: "observe",
      id: "same-token",
    });
    first.terminate();
    const third = new SelectionWorkerClient(user);
    let expired = false;
    try {
      await third.selection({ kind: "observe", id: "same-token" });
    } catch {
      expired = true;
    }
    const other = await second.selection({ kind: "observe", id: "same-token" });
    await third.close();
    await second.close();
    store.close();
    return { retained, expired, other };
  });
  expect(result.retained.selected).toBe(1);
  expect(result.expired).toBe(true);
  expect(result.other.selected).toBe(0);
});

test("selection metadata rolls back failed cache writes and account removal deletes only its membership", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const path = "/src/storage.ts",
      workerPath = "/src/selection_worker_client.ts",
      removalPath = "/src/account_removal.ts";
    const { BrowserStore } = await import(path),
      { SelectionWorkerClient } = await import(workerPath),
      { removalPreview, reviewStores } = await import(removalPath);
    const user = "D".repeat(43),
      store = await BrowserStore.open(user);
    const mail = (id: string, account: string) => ({
      core: {
        id,
        account_id: account,
        folder: "INBOX",
        sender: "Sender",
        subject: "Selection removal fixture",
        timestamp: 3,
        unread: true,
        starred: false,
      },
      text: "PRIVATE REMOVED TEXT",
    });
    await store.commit([
      ...["work", "personal"].map((id) => ({
        store: "accounts",
        key: id,
        value: { id, email: `${id}@example.test` },
      })),
      { store: "mail", key: "work-mail", value: mail("work-mail", "work") },
      {
        store: "mail",
        key: "personal-mail",
        value: mail("personal-mail", "personal"),
      },
    ]);
    const worker = new SelectionWorkerClient(user);
    await worker.selection({
      kind: "capture",
      id: "live",
      revision: 0,
      scope: { folder: "Inbox" },
      all: true,
    });
    await worker.selection({
      kind: "freeze",
      id: "live",
      expected: 0,
      target: "frozen",
    });
    let refused = false;
    try {
      await store.commit([
        {
          store: "mail",
          key: "work-mail",
          value: { ...mail("work-mail", "work"), moved: true },
        },
        {
          store: "mailAliases",
          key: "invalid",
          value: { target: () => "uncloneable" },
        },
      ]);
    } catch {
      refused = true;
    }
    const rolledBack = await worker.selection({
      kind: "observe",
      id: "frozen",
    });
    const review = removalPreview(await store.snapshot(reviewStores), "work");
    await store.removeAccount(review, false);
    const live = await worker.selection({ kind: "observe", id: "live" }, [
      "work-mail",
      "personal-mail",
    ]);
    const frozen = await worker.selection({ kind: "observe", id: "frozen" });
    const rows = [
      ...(await worker.selection({ kind: "page", id: "live", expected: 0 }))
        .rows,
      ...(await worker.selection({ kind: "page", id: "frozen", expected: 0 }))
        .rows,
    ];
    let removed = false;
    try {
      await worker.selection({
        kind: "capture",
        id: "removed",
        revision: 0,
        scope: { folder: "Inbox", account: "work" },
        all: true,
      });
    } catch {
      removed = true;
    }
    const raw = JSON.stringify({
      rows,
      journal: await store.all("mailChanges"),
    });
    await worker.close();
    store.close();
    return { refused, rolledBack, live, frozen, rows, removed, raw };
  });
  expect(result.refused).toBe(true);
  expect(result.rolledBack.available).toBe(2);
  expect(result.live.selected).toBe(1);
  expect(result.live.visible).toEqual(["personal-mail"]);
  expect(result.frozen.selected).toBe(1);
  expect(result.rows).toHaveLength(2);
  expect(result.raw).not.toContain("work");
  expect(result.raw).not.toContain("PRIVATE");
  expect(result.removed).toBe(true);
});

test("a 100000-message SQLite worker capture is bounded and independent of draft saves", async ({
  page,
}) => {
  test.setTimeout(120000); // Cardinality/concurrency contract, not a latency budget.
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const path = "/src/storage.ts",
      workerPath = "/src/selection_worker_client.ts",
      changesPath = "/src/cache_changes.ts";
    const { BrowserStore, openMailDatabase } = await import(path),
      { SelectionWorkerClient } = await import(workerPath),
      { mailMetadata } = await import(changesPath);
    const user = "L".repeat(43),
      store = await BrowserStore.open(user);
    await store.commit([
      { store: "accounts", key: "work", value: { id: "work" } },
    ]);
    const db = await openMailDatabase(user);
    // Bounded synthetic handover before opening the worker; no raw MIME is
    // needed. Metadata and cached text enter the same transaction.
    for (let start = 0; start < 100000; start += 500) {
      await new Promise<void>((resolve, reject) => {
        const tx = db.transaction(["mail", "mailMetadata"], "readwrite");
        for (let i = start; i < start + 500; i++) {
          const id = `message-${String(i).padStart(6, "0")}`;
          const value = {
            core: {
              id,
              account_id: "work",
              folder: "INBOX",
              sender: "Sender",
              subject: "Large selection fixture",
              timestamp: i,
              unread: i % 2 === 0,
              starred: false,
            },
            text: "Captured fixture text",
          };
          tx.objectStore("mail").put(value, id);
          tx.objectStore("mailMetadata").put(mailMetadata(value), id);
        }
        tx.oncomplete = () => resolve();
        tx.onabort = () => reject(tx.error);
      });
    }
    const worker = new SelectionWorkerClient(user);
    let captured = false,
      ticks = 0;
    const timer = setInterval(() => ticks++, 0);
    const reading = new Promise<void>((resolve) =>
      worker.addEventListener("activity", () => resolve(), { once: true }),
    );
    const operation = worker
      .selection(
        {
          kind: "capture",
          id: "large",
          revision: 0,
          scope: { folder: "Inbox" },
          all: true,
        },
        ["message-099999", "message-000000"],
      )
      .then((result: any) => {
        captured = true;
        return result;
      });
    // The event observes the real worker's active readonly snapshot. It cannot
    // mutate its cache, SQL selection or queue.
    await reading;
    await store.commit([
      {
        store: "drafts",
        key: "draft",
        value: {
          id: "draft",
          accountId: "work",
          body: "Saved while selection is reading",
          revision: 1,
        },
      },
    ]);
    const independent = !captured,
      draft = await store.get("drafts", "draft");
    const queued = Promise.all(
      Array.from({ length: 34 }, () =>
        worker.selection({ kind: "observe", id: "large" }).then(
          () => "accepted",
          (error: Error) =>
            error.message.includes("catching up") ? "full" : "unexpected",
        ),
      ),
    );
    const first = await operation;
    const queueResults = await queued;
    const frozen = await worker.selection({
      kind: "freeze",
      id: "large",
      expected: 0,
      target: "review",
    });
    const end = await worker.selection({
      kind: "page",
      id: "review",
      expected: 0,
      after: 99949,
    });
    clearInterval(timer);
    await worker.close();
    db.close();
    store.close();
    return {
      independent,
      queueResults,
      draft,
      first,
      frozen,
      end,
      ticks,
      bytes: JSON.stringify(first).length,
    };
  });
  expect(result.independent).toBe(true);
  expect(result.queueResults.filter((v: string) => v === "full")).toHaveLength(
    3,
  );
  expect(
    result.queueResults.filter((v: string) => v === "accepted"),
  ).toHaveLength(31);
  expect(result.draft.body).toBe("Saved while selection is reading");
  expect(result.first.selected).toBe(100000);
  expect(result.first.unread).toBe(50000);
  expect(result.first.positions["message-000000"]).toBe(99999);
  expect(result.frozen.selected).toBe(100000);
  expect(result.end.rows).toHaveLength(50);
  expect(result.end.rows[49].position).toBe(99999);
  expect(result.bytes).toBeLessThan(8192);
  expect(result.ticks).toBeGreaterThan(0);
});

test("journal overflow refreshes captured metadata without adding arrivals and merges aliases", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const storagePath = "/src/storage.ts",
      workerPath = "/src/selection_worker_client.ts";
    const { BrowserStore } = await import(storagePath),
      { SelectionWorkerClient } = await import(workerPath);
    const user = "J".repeat(43),
      store = await BrowserStore.open(user);
    const mail = (id: string, timestamp = 1) => ({
      core: {
        id,
        account_id: "work",
        folder: "INBOX",
        sender: "Sender",
        subject: "Journal fixture",
        timestamp,
        unread: true,
        starred: false,
      },
      text: "Synthetic body",
    });
    await store.commit([
      { store: "accounts", key: "work", value: { id: "work" } },
      ...["old", "target", "missing"].map((id, i) => ({
        store: "mail",
        key: id,
        value: mail(id, 3 - i),
      })),
    ]);
    const worker = new SelectionWorkerClient(user),
      scope = { folder: "Inbox" };
    await worker.selection({
      kind: "capture",
      id: "live",
      revision: 0,
      scope,
      all: true,
    });
    await worker.selection({
      kind: "change",
      id: "live",
      expected: 0,
      scope,
      change: {
        kind: "set",
        id: "target",
        selected: false,
        clear_others: false,
      },
    });
    await worker.selection({
      kind: "freeze",
      id: "live",
      expected: 1,
      target: "review",
    });
    const target = mail("target", 2);
    target.core.folder = "Archive";
    target.core.unread = false;
    target.core.starred = true;
    await store.commit([
      { store: "mail", key: "old" },
      { store: "mail", key: "target", value: target },
      {
        store: "mailAliases",
        key: "old",
        value: { alias: "old", target: "target" },
      },
      { store: "mail", key: "missing" },
    ]);
    // A single large update advances the replay floor; then ordinary updates
    // exhaust that window too. The worker sleeps throughout both handovers.
    await store.commit(
      Array.from({ length: 1100 }, (_, i) => ({
        store: "mail",
        key: `arrival-${i}`,
        value: mail(`arrival-${i}`),
      })),
    );
    for (let start = 0; start < 1100; start += 100) {
      await store.commit(
        Array.from({ length: 100 }, (_, i) => ({
          store: "mail",
          key: `arrival-${start + i}`,
          value: mail(`arrival-${start + i}`, 2),
        })),
      );
    }
    const journal = await store.all("mailChanges"),
      state = await store.get("cacheState", "mail");
    const live = await worker.selection({ kind: "observe", id: "live" }, [
      "old",
      "target",
      "missing",
      "arrival-0",
    ]);
    const frozen = await worker.selection({ kind: "observe", id: "review" });
    const rows = await worker.selection({
      kind: "page",
      id: "live",
      expected: 1,
    });
    const recapture = await worker.selection({
      kind: "capture",
      id: "live",
      revision: 2,
      scope,
      all: true,
    });
    // A dangling alias retains missing membership/account ownership.
    await store.commit([
      { store: "mail", key: "target" },
      {
        store: "mailAliases",
        key: "target",
        value: { alias: "target", target: "not-cached" },
      },
    ]);
    const dangling = await worker.selection({ kind: "observe", id: "review" }, [
      "target",
    ]);
    await Promise.all([worker.close(), worker.close()]);
    store.close();
    return { journal, state, live, frozen, rows, recapture, dangling };
  });
  expect(result.journal).toHaveLength(1024);
  expect(result.journal[0].revision).toBe(result.state.floor + 1);
  expect(result.live.total).toBe(2);
  expect(result.live.selected).toBe(2);
  expect(result.live.available).toBe(1);
  expect(result.live.visible).toEqual(["target"]);
  expect(result.live.positions.target).toBe(0);
  expect(result.live.aliases.old).toBe("target");
  expect(result.frozen.selected).toBe(2);
  expect(result.rows.rows).toEqual([
    {
      position: 0,
      id: "target",
      account: "work",
      folder: "Archive",
      unread: false,
      starred: true,
    },
  ]);
  expect(result.recapture.selected).toBe(1100);
  expect(result.dangling.selected).toBe(2);
  expect(result.dangling.available).toBe(0);
});

test("failed recapture rolls back the previous membership and revision", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const storagePath = "/src/storage.ts",
      workerPath = "/src/selection_worker_client.ts";
    const { BrowserStore, openMailDatabase } = await import(storagePath),
      { SelectionWorkerClient } = await import(workerPath);
    const user = "F".repeat(43),
      store = await BrowserStore.open(user);
    await store.commit([
      { store: "accounts", key: "work", value: { id: "work" } },
      {
        store: "mail",
        key: "original",
        value: {
          core: {
            id: "original",
            account_id: "work",
            folder: "INBOX",
            sender: "Sender",
            subject: "Original",
            timestamp: 5,
            unread: true,
            starred: false,
          },
        },
      },
    ]);
    const worker = new SelectionWorkerClient(user),
      scope = { folder: "Inbox" };
    await worker.selection({
      kind: "capture",
      id: "live",
      revision: 0,
      scope,
      all: true,
    });
    const db = await openMailDatabase(user);
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction("mailMetadata", "readwrite");
      // A corrupt second cursor row fails after the worker replaces its SQL
      // session and inserts the first row. It must roll that transaction back.
      tx.objectStore("mailMetadata").put(
        { id: "corrupt", newest: [-4, "corrupt"], oldest: [4, "corrupt"] },
        "corrupt",
      );
      tx.oncomplete = () => resolve();
      tx.onabort = () => reject(tx.error);
    });
    let failed = false;
    try {
      await worker.selection({
        kind: "capture",
        id: "live",
        revision: 1,
        scope,
        all: false,
      });
    } catch {
      failed = true;
    }
    const retained = await worker.selection({ kind: "observe", id: "live" }, [
      "original",
    ]);
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction("mailMetadata", "readwrite");
      tx.objectStore("mailMetadata").delete("corrupt");
      tx.oncomplete = () => resolve();
      tx.onabort = () => reject(tx.error);
    });
    const retried = await worker.selection({
      kind: "capture",
      id: "live",
      revision: 1,
      scope,
      all: false,
    });
    await worker.close();
    db.close();
    store.close();
    return { failed, retained, retried };
  });
  expect(result.failed).toBe(true);
  expect(result.retained.revision).toBe(0);
  expect(result.retained.selected).toBe(1);
  expect(result.retained.visible).toEqual(["original"]);
  expect(result.retried.revision).toBe(1);
  expect(result.retried.selected).toBe(0);
});

test("capture respects account, logical Sent, body search, projected filters and stable ties", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const storagePath = "/src/storage.ts",
      workerPath = "/src/selection_worker_client.ts";
    const { BrowserStore } = await import(storagePath),
      { SelectionWorkerClient } = await import(workerPath);
    const user = "Q".repeat(43),
      store = await BrowserStore.open(user);
    const mail = (
      id: string,
      account: string,
      folder: string,
      timestamp: number,
      text = "body needle",
    ) => ({
      core: {
        id,
        account_id: account,
        folder,
        sender: '"Named Sender" <sender@example.test>',
        subject: "Synthetic",
        timestamp,
        unread: false,
        starred: false,
      },
      text,
    });
    await store.commit([
      ...["work", "personal"].map((id) => ({
        store: "accounts",
        key: id,
        value: { id, sent_folder: "Sent Mail" },
      })),
      {
        store: "mailRoles",
        key: "work",
        value: { account: "work", acknowledged: ["Former Sent"] },
      },
      ...[
        mail("a", "work", "Sent Mail", 20),
        mail("b", "work", "Former Sent", 20),
        mail("c", "work", "Sent", 10),
        mail("d", "personal", "Sent", 30),
        mail("e", "work", "INBOX", 40),
        mail("f", "work", "Sent", 50, "unmatched"),
      ].map((value) => ({ store: "mail", key: value.core.id, value })),
    ]);
    const worker = new SelectionWorkerClient(user);
    const scope = {
      folder: "Sent",
      account: "work",
      query: "named needle",
      oldest: true,
    };
    const oldest = await worker.selection(
      { kind: "capture", id: "oldest", revision: 0, scope, all: true },
      ["a", "b", "c"],
    );
    const newest = await worker.selection(
      {
        kind: "capture",
        id: "newest",
        revision: 0,
        scope: { ...scope, oldest: false },
        all: true,
      },
      ["a", "b", "c"],
    );
    const projected = await worker.selection(
      {
        kind: "capture",
        id: "projected",
        revision: 0,
        scope: {
          ...scope,
          filter: "Flagged",
          projection: {
            a: { starred: true },
            b: { folder: "Archive", starred: true },
            e: { folder: "Sent", starred: true },
          },
        },
        all: true,
      },
      ["a", "b", "e"],
    );
    await worker.close();
    store.close();
    return { oldest, newest, projected };
  });
  expect(result.oldest.selected).toBe(3);
  expect(result.oldest.positions).toEqual({ a: 1, b: 2, c: 0 });
  expect(result.newest.positions).toEqual({ a: 0, b: 1, c: 2 });
  expect(result.projected.visible).toEqual(["a", "e"]);
  expect(result.projected.groups).toContainEqual({
    account: "work",
    folder: "INBOX",
    total: 1,
    unread: 0,
    starred: 0,
  });
});

test("gateway repository creates selection lazily and releases it for a fresh capture", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const storagePath = "/src/storage.ts",
      providerPath = "/src/provider.ts";
    const { BrowserStore } = await import(storagePath),
      { GatewayRepository } = await import(providerPath);
    const user = "G".repeat(43),
      store = await BrowserStore.open(user);
    await store.commit([
      {
        store: "accounts",
        key: "stable-account",
        value: { id: "stable-account", email: "owner@example.test" },
      },
      {
        store: "mail",
        key: "cached",
        value: {
          core: {
            id: "cached",
            account_id: "stable-account",
            folder: "INBOX",
            sender: "Sender",
            subject: "Gateway selection",
            timestamp: 5,
            unread: true,
            starred: false,
          },
          text: "Synthetic",
        },
      },
    ]);
    const repository = new GatewayRepository({ user_id: user }, store, () => {
      throw Error("Selection must not contact a provider");
    });
    const command = {
      kind: "capture",
      id: "live",
      revision: 0,
      scope: { folder: "Inbox", account: "stable-account" },
      all: true,
    };
    const first = await repository.selection(command, ["cached"]);
    let bounded = false;
    try {
      await repository.selection(
        { kind: "observe", id: "live" },
        Array(51).fill("cached"),
      );
    } catch {
      bounded = true;
    }
    await repository.closeSelections();
    let expired = false;
    try {
      await repository.selection({ kind: "observe", id: "live" });
    } catch {
      expired = true;
    }
    const next = await repository.selection(command);
    await repository.closeSelections();
    store.close();
    return { first, next, bounded, expired };
  });
  expect(result.first.visible).toEqual(["cached"]);
  expect(result.first.unread).toBe(1);
  expect(result.next.selected).toBe(1);
  expect(result.bounded && result.expired).toBe(true);
});
