import { test, expect, type Page } from "@playwright/test";
const profile = "E".repeat(43);
async function setup(page: Page, count = 3, seed = true) {
  await page.route("**/executor-fixture", (r) =>
    r.fulfill({
      contentType: "text/html",
      body: "<!doctype html><title>Durable executor fixture</title>",
    }),
  );
  await page.goto("/executor-fixture");
  await page.evaluate(
    async ({ profile, count, seed }) => {
      const storagePath = "/src/storage.ts",
        providerPath = "/src/provider.ts",
        journalPath = "/src/bulk_journal.ts",
        executorPath = "/src/bulk_executor.ts",
        selectionPath = "/src/selection_worker_client.ts";
      const { BrowserStore } = await import(storagePath),
        { GatewayRepository } = await import(providerPath),
        { BulkJournal } = await import(journalPath),
        { BulkExecutor } = await import(executorPath),
        { SelectionWorkerClient } = await import(selectionPath);
      const store = await BrowserStore.open(profile),
        account = {
          id: "work",
          name: "Work",
          email: "work@example.test",
          protocol: "Imap",
          host: "mail.example.test",
          port: 993,
          username: "work",
          incoming_security: "Tls",
          incoming_auth: "Password",
          smtp_host: "mail.example.test",
          smtp_port: 465,
          smtp_username: "work",
          smtp_security: "Tls",
          smtp_auth: "Automatic",
          smtp_separate_password: false,
          sent_copy: "LocalOnly",
          sent_folder: "Sent",
        };
      if (seed) {
        const changes: any[] = [
          { store: "accounts", key: account.id, value: account },
        ];
        for (let i = 0; i < count; i++) {
          const id = `m${i}`,
            core = {
              id: `work:INBOX:42.${i + 1}`,
              account_id: "work",
              remote_id: `42.${i + 1}`,
              folder: "INBOX",
              sender: "Sender <sender@example.test>",
              recipient: account.email,
              subject: `Executor letter ${i}`,
              preview: "Fictional cached mail",
              timestamp: 1000 - i,
              unread: true,
              starred: false,
              attachment_count: 0,
            };
          changes.push(
            {
              store: "mail",
              key: id,
              value: { core, localId: id, text: `Executor body ${i}` },
            },
            {
              store: "raw",
              key: id,
              value: btoa(
                `Subject: Executor letter ${i}\r\n\r\nFictional body ${i}`,
              ),
            },
          );
        }
        await store.commit(changes);
      }
      const env: any = {
        store,
        BulkJournal,
        BulkExecutor,
        profile,
        calls: [],
        unknownUid: false,
        holding: false,
      };
      let moved: any,
        serial = 0;
      const request: typeof fetch = async (input, init) => {
        const path = String(input),
          body = init?.body ? JSON.parse(String(init.body)) : undefined;
        env.calls.push({ path, body });
        await env.beforeRequest?.(path, body);
        if (path.endsWith("/probe")) return Response.json({ connected: true });
        if (path.endsWith("/flags")) return Response.json({ committed: true });
        if (path.endsWith("/move")) {
          const remote_id = `91.${++serial}`;
          moved = {
            ...body.mail,
            folder: body.folder,
            remote_id,
            id: `work:${body.folder}:${remote_id}`,
          };
          return Response.json({
            committed: true,
            remote_id: env.unknownUid ? null : remote_id,
          });
        }
        if (path.endsWith("/resolve-move"))
          return Response.json({ mail: moved });
        throw Error(`Unexpected synthetic request ${path}`);
      };
      env.repo = new GatewayRepository(
        { user_id: profile, email: "owner@example.test", csrf: "X".repeat(43) },
        store,
        request,
      );
      await env.repo.load();
      if (seed)
        await env.repo.connect(account, "fixture-password", "fixture-password");
      env.executor = new BulkExecutor(profile, env.repo);
      env.prepare = async (id: string, action: unknown) => {
        const worker = new SelectionWorkerClient(profile);
        try {
          await worker.selection({
            kind: "capture",
            id: "scope",
            revision: 0,
            scope: { folder: "Inbox" },
            all: true,
          });
          await worker.selection({
            kind: "freeze",
            id: "scope",
            expected: 0,
            target: "review",
          });
          return await worker.prepareBulk("review", 0, id, action);
        } finally {
          await worker.close();
        }
      };
      env.review = (id: string) =>
        BulkJournal.inspect(profile, (journal: any) => journal.get(id));
      env.items = (id: string, after = -1) =>
        BulkJournal.inspect(profile, (journal: any) => journal.page(id, after));
      env.hold = () => {
        let release!: () => void;
        const promise = new Promise<void>((resolve) => (release = resolve));
        let first = true;
        env.release = release;
        env.beforeRequest = async (path: string) => {
          if (first && /\/(flags|move)$/.test(path)) {
            first = false;
            env.holding = true;
            await promise;
          }
        };
      };
      (window as any).executorFixture = env;
    },
    { profile, count, seed },
  );
}

test("mail version seven closes older writers and preserves their clock without inventing cache acknowledgments", async ({
  page,
}) => {
  await page.route("**/upgrade-fixture", (r) =>
    r.fulfill({
      contentType: "text/html",
      body: "<!doctype html><title>Writer upgrade fixture</title>",
    }),
  );
  await page.goto("/upgrade-fixture");
  const r = await page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore, stores } = await import(path),
      name = `shep.mail.v1.${profile}`;
    const old = await new Promise<IDBDatabase>((resolve, reject) => {
      const r = indexedDB.open(name, 6);
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
        r.transaction!.objectStore("outgoing").createIndex("submission", "id");
      };
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error);
    });
    await new Promise<void>((resolve, reject) => {
      const tx = old.transaction(
        ["intentState", "mailIntents", "mailRoles"],
        "readwrite",
      );
      tx.objectStore("intentState").put(13, "clock");
      tx.objectStore("mailIntents").put(
        {
          id: "m0",
          account: "work",
          fields: { starred: { revision: 13, value: true, status: "applied" } },
        },
        "m0",
      );
      tx.objectStore("mailRoles").put(
        { account: "work", acknowledged: ["Keep Sent"] },
        "work",
      );
      tx.oncomplete = () => resolve();
      tx.onabort = () => reject(tx.error);
    });
    let changed = false;
    old.onversionchange = () => {
      changed = true;
      old.close();
    };
    const store = await BrowserStore.open(profile);
    let oldWriteRefused = false;
    try {
      old.transaction("mail", "readwrite");
    } catch {
      oldWriteRefused = true;
    }
    const oldOpenRefused = await new Promise<boolean>((resolve) => {
      const r = indexedDB.open(name, 6);
      r.onerror = () => resolve(r.error?.name === "VersionError");
      r.onsuccess = () => {
        r.result.close();
        resolve(false);
      };
    });
    const next = await store.intents.reserve(),
      intent = await store.get("mailIntents", "m0"),
      roles = await store.get("mailRoles", "work");
    store.close();
    return { changed, oldWriteRefused, oldOpenRefused, next, intent, roles };
  }, profile);
  expect(r.changed && r.oldWriteRefused && r.oldOpenRefused).toBe(true);
  expect(r.next).toBe(14);
  expect(r.intent.fields.starred.status).toBe("applied");
  expect(r.intent.applied).toBeUndefined();
  expect(r.roles.acknowledged).toEqual(["Keep Sent"]);
});

test("version-two migration retains an acknowledged cache gap and refuses execution without saved intent", async ({
  page,
}) => {
  await setup(page, 1);
  const r = await page.evaluate(async () => {
    const e = (window as any).executorFixture,
      name = `shep.bulk.v1.${e.profile}`;
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const open = indexedDB.open(name, 2);
      open.onupgradeneeded = () => {
        const jobs = open.result.createObjectStore("jobs", { keyPath: "id" });
        jobs.createIndex("created", ["created", "id"]);
        jobs.createIndex("state", "state");
        jobs.createIndex("queue", ["runnable", "created", "id"]);
        const items = open.result.createObjectStore("items", {
          keyPath: ["job", "position"],
        });
        items.createIndex("status", ["job", "status", "position"]);
        items.createIndex("recovery", "status");
        items.createIndex("identity", ["job", "id"], { unique: true });
        items.createIndex("cache", ["cache", "job", "position"]);
      };
      open.onsuccess = () => resolve(open.result);
      open.onerror = () => reject(open.error);
    });
    const before = {
      id: "m0",
      account: "work",
      folder: "INBOX",
      remoteId: "42.1",
      unread: true,
      starred: false,
    };
    const receipt = { before, after: { ...before, starred: true } };
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(["jobs", "items"], "readwrite");
      tx.objectStore("jobs").put({
        id: "legacy",
        action: { kind: "flags", starred: true },
        state: "ready",
        created: 1,
        revision: 4,
        total: 1,
        staged: 1,
        lastPosition: 0,
        paused: false,
        undo: false,
        pendingCache: 1,
        runnable: 0,
        counts: {
          pending: 0,
          running: 0,
          done: 1,
          undo_running: 0,
          restored: 0,
          failed: 0,
          uncertain: 0,
          missing: 0,
        },
      });
      tx.objectStore("items").put({
        job: "legacy",
        position: 0,
        id: "m0",
        account: "work",
        original: before,
        status: "done",
        phase: "forward",
        attempt: "legacy-attempt",
        receipt,
        cache: 1,
      });
      tx.oncomplete = () => resolve();
      tx.onabort = () => reject(tx.error);
    });
    db.close();
    const migrated = await e.review("legacy");
    let refused = false;
    try {
      await e.executor.run();
    } catch {
      refused = true;
    }
    return {
      migrated,
      refused,
      after: await e.review("legacy"),
      items: await e.items("legacy"),
      receipt,
      writes: e.calls.filter((c: any) => c.path.endsWith("/flags")),
    };
  });
  expect(r.refused).toBe(true);
  expect(r.migrated).toMatchObject({
    pendingCache: 1,
    counts: { done: 1, skipped: 0 },
  });
  expect(r.after).toEqual(r.migrated);
  expect(r.items[0].receipt).toEqual(r.receipt);
  expect(r.writes).toHaveLength(0);
});

test("mail rows and cache acknowledgments commit atomically, retain refresh values and survive aliases", async ({
  page,
}) => {
  await setup(page, 1);
  const r = await page.evaluate(async () => {
    const e = (window as any).executorFixture,
      s = e.store,
      lease = await s.intents.register("m0", { unread: false });
    const mail = await s.get("mail", "m0");
    mail.core.unread = false;
    const before = await s.snapshot([
      "mail",
      "mailIntents",
      "mailMetadata",
      "cacheState",
    ]);
    let refused = false;
    try {
      await s.commit([{ store: "mail", key: "m0", value: mail }], {
        ...lease,
        fields: { unread: true },
      });
    } catch {
      refused = true;
    }
    const rollback =
      JSON.stringify(before) ===
      JSON.stringify(
        await s.snapshot(["mail", "mailIntents", "mailMetadata", "cacheState"]),
      );
    await s.commit([{ store: "mail", key: "m0", value: mail }], lease);
    // A later provider refresh can change this value without erasing the
    // acknowledgment that prevents an older receipt from being replayed.
    mail.core.unread = true;
    await s.commit([{ store: "mail", key: "m0", value: mail }]);
    const uncached = await s.intents.uncached(lease);
    const newer = await s.intents.register("m0", { unread: false });
    mail.core.unread = false;
    await s.commit([{ store: "mail", key: "m0", value: mail }], newer);
    const snapshot = await s.snapshot([
      "mail",
      "mailIntents",
      "mailMetadata",
      "cacheState",
    ]);
    mail.core.starred = true;
    let stale = false;
    try {
      await s.commit([{ store: "mail", key: "m0", value: mail }], lease);
    } catch {
      stale = true;
    }
    const staleRollback =
      JSON.stringify(snapshot) ===
      JSON.stringify(
        await s.snapshot(["mail", "mailIntents", "mailMetadata", "cacheState"]),
      );
    const current = await s.get("mail", "m0");
    await s.commit([
      { store: "mail", key: "copy", value: { ...current, localId: "copy" } },
    ]);
    const copy = await s.intents.register("copy", { starred: true }),
      target = await s.get("mail", "copy");
    target.core.starred = true;
    await s.commit([{ store: "mail", key: "copy", value: target }], copy);
    await s.commit([
      {
        store: "mailAliases",
        key: "m0",
        value: { alias: "m0", target: "copy" },
      },
      { store: "mail", key: "m0" },
    ]);
    return {
      refused,
      rollback,
      uncached,
      stale,
      staleRollback,
      merged: await s.get("mailIntents", "copy"),
      newer,
      copy,
    };
  });
  expect(r.refused && r.rollback && r.stale && r.staleRollback).toBe(true);
  expect(r.uncached).toEqual({});
  expect(r.merged.applied).toEqual({
    unread: r.newer.revision,
    starred: r.copy.revision,
  });
});

test("fully superseded fields are skipped and never counted as changed or undone", async ({
  page,
}) => {
  await setup(page, 1);
  const r = await page.evaluate(async () => {
    const e = (window as any).executorFixture,
      review = await e.prepare("group", { kind: "flags", starred: true });
    await e.executor.decide("group", review.revision, "approve");
    await e.repo.mutate("m0", { starred: false });
    await e.executor.run();
    const job = await e.review("group"),
      items = await e.items("group");
    await e.executor.decide("group", job.revision, "undo");
    await e.executor.run();
    return {
      job,
      items,
      done: await e.review("group"),
      calls: e.calls.filter((c: any) => c.path.endsWith("/flags")),
    };
  });
  expect(r.job.counts).toMatchObject({
    done: 0,
    skipped: 1,
    failed: 0,
    uncertain: 0,
  });
  expect(r.items[0].receipt).toBeUndefined();
  expect(r.done.counts.restored).toBe(0);
  expect(r.calls).toHaveLength(1);
});

test("a pending cache receipt cannot undo a later acknowledged move", async ({
  page,
}) => {
  await setup(page, 1);
  const r = await page.evaluate(async () => {
    const e = (window as any).executorFixture,
      review = await e.prepare("group", {
        kind: "move",
        folder: "Archive",
        account: null,
      });
    await e.executor.decide("group", review.revision, "approve");
    const saved = e.BulkJournal.prototype.cacheSaved;
    let fail = true;
    e.BulkJournal.prototype.cacheSaved = async function (...args: any[]) {
      if (fail) throw Error("Synthetic journal checkpoint failure");
      return saved.apply(this, args);
    };
    await e.executor.run().catch(() => {});
    fail = false;
    const pending = await e.review("group");
    await e.repo.mutate("m0", { folder: "Trash" });
    await new e.BulkExecutor(e.profile, e.repo).run();
    const repaired = await e.review("group");
    await e.executor.decide("group", repaired.revision, "undo");
    await e.executor.run();
    return {
      pending,
      mail: await e.store.get("mail", "m0"),
      job: await e.review("group"),
      moves: e.calls.filter((c: any) => c.path.endsWith("/move")),
    };
  });
  expect(r.pending.pendingCache).toBe(1);
  expect(r.mail.core).toMatchObject({ folder: "Trash", remote_id: "91.2" });
  expect(r.job).toMatchObject({
    pendingCache: 0,
    counts: { skipped: 1, restored: 0 },
  });
  expect(r.moves).toHaveLength(2);
});

test("changed physical sources and foreign profiles cannot dispatch the reviewed group", async ({
  page,
}) => {
  await setup(page, 1);
  const r = await page.evaluate(async () => {
    const e = (window as any).executorFixture;
    let foreign = false;
    try {
      new e.BulkExecutor("F".repeat(43), e.repo);
    } catch {
      foreign = true;
    }
    const review = await e.prepare("group", { kind: "flags", unread: false });
    await e.executor.decide("group", review.revision, "approve");
    await e.repo.mutate("m0", { folder: "Archive" });
    await e.executor.run();
    return {
      foreign,
      job: await e.review("group"),
      intent: await e.store.get("mailIntents", "m0"),
      flags: e.calls.filter((c: any) => c.path.endsWith("/flags")),
    };
  });
  expect(r.foreign).toBe(true);
  expect(r.job.counts).toMatchObject({ failed: 1, done: 0, uncertain: 0 });
  expect(r.intent.fields.unread.status).toBe("failed");
  expect(r.flags).toHaveLength(0);
});

test("definite pre-dispatch failures retire intent and retry once; ambiguous failures never automatically retry", async ({
  page,
}) => {
  await setup(page, 1);
  const r = await page.evaluate(async () => {
    const e = (window as any).executorFixture,
      review = await e.prepare("group", { kind: "flags", starred: true });
    await e.executor.decide("group", review.revision, "approve");
    const attach = e.BulkJournal.prototype.attachIntent;
    let fail = true;
    e.BulkJournal.prototype.attachIntent = async function (...args: any[]) {
      if (fail) throw Error("Synthetic attach failure");
      return attach.apply(this, args);
    };
    await e.executor.run();
    const rejected = await e.review("group"),
      intent = await e.store.get("mailIntents", "m0");
    fail = false;
    await e.executor.retry("group", rejected.revision, 0);
    await e.executor.run();
    const done = await e.review("group");
    const next = await e.prepare("unknown", { kind: "flags", unread: false });
    await e.executor.decide("unknown", next.revision, "approve");
    e.beforeRequest = async (path: string) => {
      if (path.endsWith("/flags")) throw Error("Synthetic lost response");
    };
    await e.executor.run();
    const unknown = await e.review("unknown");
    let retryRefused = false;
    try {
      await e.executor.retry("unknown", unknown.revision, 0);
    } catch {
      retryRefused = true;
    }
    await new e.BulkExecutor(e.profile, e.repo).run();
    return {
      rejected,
      intent,
      done,
      unknown,
      retryRefused,
      writes: e.calls.filter((c: any) => c.path.endsWith("/flags")),
    };
  });
  expect(r.rejected.counts.failed).toBe(1);
  expect(r.intent.fields.starred.status).toBe("failed");
  expect(r.done.counts.done).toBe(1);
  expect(r.unknown).toMatchObject({
    paused: true,
    counts: { uncertain: 1, done: 0 },
  });
  expect(r.retryRefused).toBe(true);
  expect(r.writes).toHaveLength(2);
});

test("frozen full captures execute in bounded pages and Undo preserves newer same-value intent", async ({
  page,
}) => {
  await setup(page, 125);
  const result = await page.evaluate(async () => {
    const e = (window as any).executorFixture;
    const review = await e.prepare("group", {
      kind: "flags",
      unread: false,
      starred: true,
    });
    await e.executor.decide("group", review.revision, "approve");
    // Newer input owns Star even though it has the group's same target value.
    await e.repo.mutate("m0", { starred: true });
    const forward = await e.executor.run(),
      job = await e.review("group"),
      first = await e.items("group");
    const current = await e.store.get("mail", "m0");
    await e.executor.decide("group", job.revision, "undo");
    const inverse = await e.executor.run(),
      done = await e.review("group"),
      last = await e.items("group", 99);
    const after = await e.store.get("mail", "m0"),
      ordinary = await e.store.get("mail", "m124");
    return {
      forward,
      inverse,
      job,
      done,
      first,
      last,
      current,
      after,
      ordinary,
      writes: e.calls.filter((c: any) => c.path.endsWith("/flags")),
    };
  });
  expect(result.forward.steps).toBe(125);
  expect(result.inverse.steps).toBe(125);
  expect(result.first).toHaveLength(50);
  expect(result.last).toHaveLength(25);
  expect(result.first[0].intent.fields).toEqual({ unread: false });
  expect(result.current.core).toMatchObject({ unread: false, starred: true });
  expect(result.after.core).toMatchObject({ unread: true, starred: true });
  expect(result.ordinary.core).toMatchObject({ unread: true, starred: false });
  expect(result.done.counts).toMatchObject({
    restored: 125,
    done: 0,
    uncertain: 0,
  });
  expect(result.done.pendingCache).toBe(0);
  expect(result.writes).toHaveLength(251);
});

test("Undo while MOVE is owned reverses its acknowledged UID and cancels unsent membership", async ({
  page,
}) => {
  await setup(page);
  await page.evaluate(async () => {
    const e = (window as any).executorFixture,
      r = await e.prepare("group", {
        kind: "move",
        folder: "Archive",
        account: null,
      });
    await e.executor.decide("group", r.revision, "approve");
    e.hold();
    e.running = e.executor.run();
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).executorFixture.holding))
    .toBe(true);
  const result = await page.evaluate(async () => {
    const e = (window as any).executorFixture,
      current = await e.review("group");
    await e.executor.decide("group", current.revision, "undo");
    e.release();
    await e.running;
    return {
      job: await e.review("group"),
      mail: await e.store.all("mail"),
      moves: e.calls.filter((c: any) => c.path.endsWith("/move")),
    };
  });
  expect(result.moves).toHaveLength(2);
  expect(result.moves[0].body.mail.remote_id).toBe("42.1");
  expect(result.moves[1].body.mail.remote_id).toBe("91.1");
  expect(result.moves[1].body.folder).toBe("INBOX");
  expect(result.job).toMatchObject({
    undo: true,
    pendingCache: 0,
    counts: { restored: 1, pending: 2 },
  });
  expect(result.mail.every((m: any) => m.core.folder === "INBOX")).toBe(true);
});

test("cache failure retains receipts and a reopened executor repairs without repeating the server write", async ({
  page,
}) => {
  await setup(page, 1);
  const result = await page.evaluate(async () => {
    const e = (window as any).executorFixture,
      r = await e.prepare("group", { kind: "flags", starred: true });
    await e.executor.decide("group", r.revision, "approve");
    const commit = e.store.commit.bind(e.store);
    let fail = true;
    e.store.commit = async (changes: any[], intent: any) => {
      if (fail && intent) throw Error("Synthetic full cache disk");
      return commit(changes, intent);
    };
    let failed = false;
    try {
      await e.executor.run();
    } catch {
      failed = true;
    }
    const pending = await e.review("group"),
      before = await e.store.get("mail", "m0");
    fail = false;
    const resumed = await new e.BulkExecutor(e.profile, e.repo).run(),
      done = await e.review("group"),
      after = await e.store.get("mail", "m0"),
      intent = await e.store.get("mailIntents", "m0");
    return {
      failed,
      pending,
      before,
      resumed,
      done,
      after,
      intent,
      writes: e.calls.filter((c: any) => c.path.endsWith("/flags")).length,
    };
  });
  expect(result.failed).toBe(true);
  expect(result.pending).toMatchObject({
    pendingCache: 1,
    counts: { done: 1, uncertain: 0 },
  });
  expect(result.before.core.starred).toBe(false);
  expect(result.after.core.starred).toBe(true);
  expect(result.resumed).toMatchObject({ steps: 0, repairs: 1 });
  expect(result.writes).toBe(1);
  expect(result.done.pendingCache).toBe(0);
  expect(result.intent.applied.starred).toBe(result.pending.forwardIntent);
});

test("older cache repair keeps newer acknowledged fields even when intent completion lost its reply", async ({
  page,
}) => {
  await setup(page, 1);
  const result = await page.evaluate(async () => {
    const e = (window as any).executorFixture,
      r = await e.prepare("group", {
        kind: "flags",
        starred: true,
        unread: false,
      });
    await e.executor.decide("group", r.revision, "approve");
    const commit = e.store.commit.bind(e.store);
    let fail = true;
    e.store.commit = async (changes: any[], intent: any) => {
      if (fail && intent) throw Error("Synthetic cache failure");
      return commit(changes, intent);
    };
    await e.executor.run().catch(() => {});
    fail = false;
    const finish = e.store.intents.finish.bind(e.store.intents);
    e.store.intents.finish = async () => {
      throw Error("Synthetic lost intent completion");
    };
    // This later server/cache write is known even though finish() never ran.
    const warning = await e.repo
      .mutate("m0", { starred: false })
      .catch((error: any) => ({ committed: error.committed }));
    const pending = await e.store.get("mailIntents", "m0");
    e.store.intents.finish = finish;
    await new e.BulkExecutor(e.profile, e.repo).run();
    const after = await e.store.get("mail", "m0"),
      done = await e.review("group"),
      intent = await e.store.get("mailIntents", "m0");
    await e.executor.decide("group", done.revision, "undo");
    await e.executor.run();
    return {
      warning,
      pending,
      after,
      intent,
      restored: await e.store.get("mail", "m0"),
      writes: e.calls.filter((c: any) => c.path.endsWith("/flags")),
    };
  });
  expect(result.warning.committed).toBe(true);
  expect(result.pending.fields.starred.status).toBe("pending");
  expect(result.after.core).toMatchObject({ starred: false, unread: false });
  expect(result.restored.core).toMatchObject({ starred: false, unread: true });
  expect(result.intent.applied.starred).toBe(
    result.pending.fields.starred.revision,
  );
  expect(result.writes).toHaveLength(3);
  expect(result.writes[2].body.starred).toBeUndefined();
});

test("lost journal acknowledgment and unknown destination recovery cannot repeat MOVE", async ({
  page,
}) => {
  await setup(page, 1);
  const result = await page.evaluate(async () => {
    const e = (window as any).executorFixture,
      r = await e.prepare("group", {
        kind: "move",
        folder: "Archive",
        account: null,
      });
    await e.executor.decide("group", r.revision, "approve");
    e.unknownUid = true;
    const settle = e.BulkJournal.prototype.settle;
    let first = true;
    e.BulkJournal.prototype.settle = async function (...args: any[]) {
      const result = await settle.apply(this, args);
      if (first && args[3].kind === "committed") {
        first = false;
        throw Error("Synthetic lost journal reply");
      }
      return result;
    };
    await e.executor.run();
    const job = await e.review("group"),
      items = await e.items("group"),
      mail = await e.store.get("mail", "m0");
    await new e.BulkExecutor(e.profile, e.repo).run();
    return { job, items, mail, calls: e.calls };
  });
  expect(result.job).toMatchObject({
    pendingCache: 0,
    counts: { done: 1, uncertain: 0 },
  });
  expect(result.items[0].receipt.after.remoteId).toBe("91.1");
  expect(result.mail.core).toMatchObject({
    folder: "Archive",
    remote_id: "91.1",
  });
  expect(result.mail.pendingMove).toBeUndefined();
  expect(
    result.calls.filter((c: any) => c.path.endsWith("/move")),
  ).toHaveLength(1);
  expect(
    result.calls.filter((c: any) => c.path.endsWith("/resolve-move")),
  ).toHaveLength(1);
});

test("graceful stop finishes the owned receipt and a fresh executor resumes only queued work", async ({
  page,
}) => {
  await setup(page);
  await page.evaluate(async () => {
    const e = (window as any).executorFixture,
      r = await e.prepare("group", { kind: "flags", starred: true });
    await e.executor.decide("group", r.revision, "approve");
    e.hold();
    e.running = e.executor.run();
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).executorFixture.holding))
    .toBe(true);
  const result = await page.evaluate(async () => {
    const e = (window as any).executorFixture;
    e.executor.stop();
    e.release();
    await e.running;
    const stopped = await e.review("group");
    await new e.BulkExecutor(e.profile, e.repo).run();
    return {
      stopped,
      done: await e.review("group"),
      writes: e.calls.filter((c: any) => c.path.endsWith("/flags")),
    };
  });
  expect(result.stopped).toMatchObject({
    pendingCache: 0,
    counts: { done: 1, pending: 2 },
  });
  expect(result.done.counts.done).toBe(3);
  expect(result.writes).toHaveLength(3);
});

test("a second tab cannot recover a live executor and tab loss leaves an unconfirmed step", async ({
  page,
  context,
}) => {
  await setup(page, 1);
  await page.evaluate(async () => {
    const e = (window as any).executorFixture,
      r = await e.prepare("group", { kind: "flags", starred: true });
    await e.executor.decide("group", r.revision, "approve");
    e.hold();
    e.running = e.executor.run().catch(() => {});
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).executorFixture.holding))
    .toBe(true);
  const other = await context.newPage();
  await setup(other, 1, false);
  const blocked = await other.evaluate(async () => {
    const e = (window as any).executorFixture;
    let refused = false;
    try {
      await e.executor.run();
    } catch {
      refused = true;
    }
    return { refused, job: await e.review("group") };
  });
  expect(blocked.refused).toBe(true);
  expect(blocked.job.counts.running).toBe(1);
  expect(blocked.job.counts.uncertain).toBe(0);
  await page.close();
  // Closing the page completes before Chromium releases its Web Lock. Observe
  // that specific ownership ending before asking the replacement to recover.
  await expect
    .poll(() =>
      other.evaluate(async () => {
        const e = (window as any).executorFixture;
        return !(await navigator.locks.query()).held?.some(
          (lock) => lock.name === `shep.bulk.v1.${e.profile}`,
        );
      }),
    )
    .toBe(true);
  const recovered = await other.evaluate(async () => {
    const e = (window as any).executorFixture,
      run = await e.executor.run();
    return {
      run,
      job: await e.review("group"),
      writes: e.calls.filter((c: any) => c.path.endsWith("/flags")),
    };
  });
  expect(recovered.run.steps).toBe(0);
  expect(recovered.job).toMatchObject({
    paused: true,
    counts: { uncertain: 1, running: 0 },
  });
  expect(recovered.writes).toHaveLength(0);
  await other.close();
});
