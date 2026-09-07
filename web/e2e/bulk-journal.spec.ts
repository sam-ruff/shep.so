import { test, expect } from "@playwright/test";

test("a 100000-message frozen export keeps draft saves independent and transfer pages bounded", async ({
  page,
}) => {
  test.setTimeout(120000);
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const sp = "/src/storage.ts",
      wp = "/src/selection_worker_client.ts",
      bp = "/src/bulk_journal.ts";
    const { openMailDatabase, BrowserStore } = await import(sp),
      { SelectionWorkerClient } = await import(wp),
      { BulkJournal } = await import(bp);
    const user = "Q".repeat(43),
      db = await openMailDatabase(user);
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(["accounts", "mailMetadata"], "readwrite");
      tx.oncomplete = () => resolve();
      tx.onabort = () => reject(tx.error);
      tx.objectStore("accounts").put(
        { id: "work", email: "work@example.test" },
        "work",
      );
      for (let i = 0; i < 100000; i++) {
        const id = `m${String(i).padStart(6, "0")}`,
          core = {
            id,
            account_id: "work",
            remote_id: `1.${i + 1}`,
            folder: "INBOX",
            sender: "sender@example.test",
            recipient: "work@example.test",
            subject: "Synthetic cached mail",
            preview: "",
            timestamp: 100000 - i,
            unread: true,
            starred: false,
            attachment_count: 0,
          };
        tx.objectStore("mailMetadata").put(
          {
            id,
            core,
            moved: false,
            newest: [-core.timestamp, id],
            oldest: [core.timestamp, id],
          },
          id,
        );
      }
    });
    db.close();
    const worker = new SelectionWorkerClient(user),
      cache = await BrowserStore.open(user);
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
      target: "review",
    });
    let started!: () => void,
      finished = false;
    const snapshot = new Promise<void>((r) => (started = r));
    worker.addEventListener("activity", () => started(), { once: true });
    const preparing = worker
      .prepareBulk("review", 0, "large", { kind: "flags", unread: false })
      .then((job: any) => {
        finished = true;
        return job;
      });
    await snapshot;
    await cache.commit([
      {
        store: "drafts",
        key: "free",
        value: { id: "free", body: "Saved during export" },
      },
    ]);
    const savedWhileExporting = !finished,
      job = await preparing;
    await worker.close();
    cache.close();
    return BulkJournal.own(user, async (j: any) => ({
      job,
      savedWhileExporting,
      first: await j.page("large"),
      last: await j.page("large", 99949),
    }));
  });
  expect(result.savedWhileExporting).toBe(true);
  expect(result.job).toMatchObject({
    state: "review",
    total: 100000,
    staged: 100000,
    counts: { pending: 100000 },
  });
  expect(result.first).toHaveLength(50);
  expect(result.last).toHaveLength(50);
  expect(result.last.at(-1)).toMatchObject({ id: "m099999", position: 99999 });
});

test("abrupt owner-tab closure preserves an uncertain inverse and its forward receipt", async ({
  page,
  context,
}) => {
  await page.goto("/preview.html");
  await page.evaluate(async () => {
    const path = "/src/bulk_journal.ts",
      { BulkJournal } = await import(path);
    let started!: () => void;
    const ready = new Promise<void>((r) => (started = r));
    (window as any).owned = BulkJournal.own("R".repeat(43), async (j: any) => {
      const original = {
        id: "m",
        account: "work",
        folder: "INBOX",
        remoteId: "1.1",
        unread: true,
        starred: false,
      };
      async function* rows() {
        yield [{ position: 0, id: "m", account: "work", original }];
      }
      const review = await j.prepare(
        "group",
        { kind: "move", folder: "Archive", account: null },
        1,
        rows(),
      );
      await j.decide("group", review.revision, "approve");
      const forward = await j.claim("group");
      const done = await j.settle("group", 0, forward.attempt, {
        kind: "committed",
        receipt: {
          before: original,
          after: { ...original, folder: "Archive", remoteId: "4.82" },
        },
      });
      await j.decide("group", done.revision, "undo");
      await j.claim("group");
      started();
      await new Promise(() => {});
    });
    await ready;
  });
  const other = await context.newPage();
  await other.goto("/preview.html");
  await page.close();
  // Chromium's page-close acknowledgment can precede Web Lock release. Observe
  // actual ownership ending; do not steal the lock or hide a live-owner refusal.
  await expect
    .poll(() =>
      other.evaluate(
        async () =>
          (await navigator.locks.query()).held?.some(
            (lock) => lock.name === `shep.bulk.v1.${"R".repeat(43)}`,
          ) ?? false,
      ),
    )
    .toBe(false);
  const result = await other.evaluate(async () => {
    const path = "/src/bulk_journal.ts",
      { BulkJournal } = await import(path);
    return BulkJournal.own("R".repeat(43), async (j: any) => ({
      job: await j.get("group"),
      item: (await j.page("group"))[0],
      claim: await j.claim("group"),
    }));
  });
  expect(result.job).toMatchObject({
    undo: true,
    paused: true,
    counts: { uncertain: 1, undo_running: 0, done: 0 },
  });
  expect(result.item).toMatchObject({
    phase: "undo",
    status: "uncertain",
    receipt: { after: { folder: "Archive", remoteId: "4.82" } },
  });
  expect(result.claim).toBeNull();
});

test("frozen worker membership becomes exact durable metadata without new arrivals", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const sp = "/src/storage.ts",
      wp = "/src/selection_worker_client.ts",
      bp = "/src/bulk_journal.ts";
    const { BrowserStore } = await import(sp),
      { SelectionWorkerClient } = await import(wp),
      { BulkJournal } = await import(bp);
    const user = "J".repeat(43),
      cache = await BrowserStore.open(user);
    const mail = (i: number) => ({
      core: {
        id: `m${i}`,
        account_id: "work",
        remote_id: `3.${i + 1}`,
        folder: "INBOX",
        sender: "sender@example.test",
        recipient: "work@example.test",
        subject: `Message ${i}`,
        preview: "Fixture",
        timestamp: 1000 - i,
        unread: true,
        starred: false,
        attachment_count: 0,
      },
      text: "private-body-must-not-enter-journal",
    });
    await cache.commit([
      {
        store: "accounts",
        key: "work",
        value: { id: "work", email: "work@example.test" },
      },
      ...Array.from({ length: 125 }, (_, i) => ({
        store: "mail",
        key: `m${i}`,
        value: mail(i),
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
      change: { kind: "set", id: "m51", selected: false, clear_others: false },
    });
    await worker.selection({
      kind: "freeze",
      id: "live",
      expected: 1,
      target: "review",
    });
    await worker.selection({
      kind: "change",
      id: "live",
      expected: 1,
      scope,
      change: { kind: "clear" },
    });
    const adopted = mail(0);
    adopted.core.id = "adopted";
    adopted.core.folder = "Archive";
    adopted.core.remote_id = "5.42";
    await cache.commit([
      { store: "mail", key: "m125", value: mail(125) },
      { store: "mail", key: "m1" },
      { store: "mail", key: "m0" },
      { store: "mail", key: "adopted", value: adopted },
      {
        store: "mailAliases",
        key: "m0",
        value: { alias: "m0", target: "adopted" },
      },
    ]);
    const job = await worker.prepareBulk("review", 0, "group", {
      kind: "flags",
      unread: false,
    });
    await worker.close();
    cache.close();
    return BulkJournal.own(user, async (j: any) => {
      const pages = [];
      let after = -1;
      for (;;) {
        const rows = await j.page("group", after);
        if (!rows.length) break;
        pages.push(rows);
        after = rows.at(-1).position;
      }
      return {
        job,
        reopened: await j.get("group"),
        pages,
        claim: await j.claim("group"),
      };
    });
  });
  expect(result.job.state).toBe("review");
  expect(result.job.total).toBe(124);
  expect(result.reopened.counts).toMatchObject({
    pending: 123,
    missing: 1,
    running: 0,
  });
  expect(result.claim).toBeNull();
  expect(result.pages.map((p: any[]) => p.length)).toEqual([50, 50, 24]);
  const rows = result.pages.flat();
  expect(rows.map((r: any) => r.id)).not.toContain("m51");
  expect(rows.map((r: any) => r.id)).not.toContain("m125");
  expect(rows[0].original).toMatchObject({
    id: "adopted",
    folder: "Archive",
    remoteId: "5.42",
  });
  expect(rows[1]).toMatchObject({
    id: "m1",
    status: "missing",
    original: null,
  });
  expect(JSON.stringify(result)).not.toContain("private-body");
});

test("Undo waits for a running receipt and uses acknowledged destination identities", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const bp = "/src/bulk_journal.ts",
      { BulkJournal } = await import(bp);
    return BulkJournal.own("K".repeat(43), async (j: any) => {
      const original = (i: number) => ({
        id: `m${i}`,
        account: "work",
        folder: "INBOX",
        remoteId: `2.${i + 1}`,
        unread: true,
        starred: false,
      });
      async function* chunks() {
        yield [0, 1, 2].map((i) => ({
          position: i,
          id: `m${i}`,
          account: "work",
          original: original(i),
        }));
      }
      const review = await j.prepare(
        "group",
        { kind: "move", folder: "Archive", account: null },
        3,
        chunks(),
      );
      const approved = await j.decide("group", review.revision, "approve");
      let stale = false;
      try {
        await j.decide("group", review.revision, "undo");
      } catch {
        stale = true;
      }
      const forward = await j.claim("group"),
        inFlight = await j.get("group");
      const other = await j.prepare(
        "other",
        { kind: "flags", starred: true },
        3,
        chunks(),
      );
      await j.decide("other", other.revision, "approve");
      const otherWhilePending = await j.claim("other");
      await j.decide("group", inFlight.revision, "undo");
      const whilePending = await j.claim("group");
      const destination = {
        ...original(0),
        folder: "Archive",
        remoteId: "9.81",
      };
      await j.settle("group", 0, forward.attempt, {
        kind: "committed",
        receipt: { before: original(0), after: destination },
      });
      let duplicate = false;
      try {
        await j.settle("group", 0, forward.attempt, {
          kind: "committed",
          receipt: { before: original(0), after: destination },
        });
      } catch {
        duplicate = true;
      }
      const inverse = await j.claim("group");
      let obsolete = false;
      try {
        await j.settle("group", 0, inverse.attempt, {
          kind: "committed",
          receipt: { before: original(0), after: original(0) },
        });
      } catch {
        obsolete = true;
      }
      const inverseFailure = await j.settle("group", 0, inverse.attempt, {
        kind: "rejected",
        error: "The server refused the inverse. Retry Undo.",
      });
      await j.retry("group", inverseFailure.revision, 0);
      const retriedInverse = await j.claim("group");
      await j.settle("group", 0, retriedInverse.attempt, {
        kind: "committed",
        receipt: {
          before: destination,
          after: { ...original(0), remoteId: "2.909" },
        },
      });
      return {
        approved,
        stale,
        duplicate,
        obsolete,
        whilePending,
        otherWhilePending,
        inverse,
        final: await j.get("group"),
        rows: await j.page("group"),
        next: await j.claim("group"),
      };
    });
  });
  expect(result.stale && result.duplicate && result.obsolete).toBe(true);
  expect(result.whilePending).toBeNull();
  expect(result.otherWhilePending).toBeNull();
  expect(result.inverse.receipt.after.remoteId).toBe("9.81");
  expect(result.rows[0].inverse.after.remoteId).toBe("2.909");
  expect(result.final.counts).toMatchObject({
    pending: 2,
    restored: 1,
    running: 0,
    done: 0,
  });
  expect(result.final.undo).toBe(true);
  expect(result.next).toBeNull();
});

test("interrupted staging and abandoned operations survive reopen without repetition", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const bp = "/src/bulk_journal.ts",
      { BulkJournal } = await import(bp),
      user = "L".repeat(43);
    const row = {
      position: 0,
      id: "m",
      account: "work",
      original: {
        id: "m",
        account: "work",
        folder: "INBOX",
        remoteId: "1.1",
        unread: true,
        starred: false,
      },
    };
    let old: any, running: any;
    await BulkJournal.own(user, async (j: any) => {
      old = j;
      async function* broken() {
        yield [row];
        throw Error("fixture interruption");
      }
      try {
        await j.prepare(
          "partial",
          { kind: "flags", starred: true },
          2,
          broken(),
        );
      } catch {}
      async function* one() {
        yield [row];
      }
      const review = await j.prepare(
        "running",
        { kind: "flags", unread: false },
        1,
        one(),
      );
      await j.decide("running", review.revision, "approve");
      running = await j.claim("running");
    });
    let expired = false;
    try {
      await old.get("running");
    } catch {
      expired = true;
    }
    return BulkJournal.own(user, async (j: any) => {
      const partial = await j.get("partial"),
        abandoned = await j.get("running");
      let partialRefused = false,
        retryRefused = false,
        lateRefused = false,
        replaceRefused = false;
      try {
        await j.decide("partial", partial.revision, "approve");
      } catch {
        partialRefused = true;
      }
      try {
        await j.retry("running", abandoned.revision, 0);
      } catch {
        retryRefused = true;
      }
      try {
        await j.settle("running", 0, running.attempt, {
          kind: "rejected",
          error: "late result",
        });
      } catch {
        lateRefused = true;
      }
      async function* one() {
        yield [row];
      }
      try {
        await j.prepare("partial", { kind: "flags", unread: false }, 1, one());
      } catch {
        replaceRefused = true;
      }
      return {
        expired,
        partial,
        abandoned,
        partialRefused,
        retryRefused,
        lateRefused,
        replaceRefused,
        claim: await j.claim("running"),
        row: (await j.page("running"))[0],
      };
    });
  });
  expect(
    result.expired &&
      result.partialRefused &&
      result.retryRefused &&
      result.lateRefused &&
      result.replaceRefused,
  ).toBe(true);
  expect(result.partial).toMatchObject({
    state: "interrupted",
    staged: 1,
    total: 2,
  });
  expect(result.abandoned).toMatchObject({
    paused: true,
    counts: { uncertain: 1, running: 0 },
  });
  expect(result.row).toMatchObject({ status: "uncertain", phase: "forward" });
  expect(result.claim).toBeNull();
});

test("another tab cannot steal group ownership while an independent draft saves", async ({
  page,
  context,
}) => {
  await page.goto("/preview.html");
  await page.evaluate(async () => {
    const bp = "/src/bulk_journal.ts",
      { BulkJournal } = await import(bp);
    let started!: () => void;
    const ready = new Promise<void>((r) => (started = r));
    (window as any).owned = BulkJournal.own("M".repeat(43), async (j: any) => {
      const original = {
        id: "m",
        account: "work",
        folder: "INBOX",
        remoteId: "1.1",
        unread: true,
        starred: false,
      };
      async function* chunks() {
        yield [{ position: 0, id: "m", account: "work", original }];
      }
      const review = await j.prepare(
        "group",
        { kind: "flags", unread: false },
        1,
        chunks(),
      );
      await j.decide("group", review.revision, "approve");
      const item = await j.claim("group");
      started();
      await new Promise<void>((r) => ((window as any).release = r));
      await j.settle("group", 0, item.attempt, {
        kind: "committed",
        receipt: { before: original, after: { ...original, unread: false } },
      });
    });
    await ready;
  });
  const other = await context.newPage();
  await other.goto("/preview.html");
  const result = await other.evaluate(async () => {
    const bp = "/src/bulk_journal.ts",
      sp = "/src/storage.ts",
      { BulkJournal } = await import(bp),
      { BrowserStore } = await import(sp),
      user = "M".repeat(43);
    let denied = false;
    try {
      await BulkJournal.own(user, async () => {});
    } catch (e) {
      denied = String(e).includes("another tab");
    }
    const store = await BrowserStore.open(user);
    await store.commit([
      {
        store: "drafts",
        key: "independent",
        value: { id: "independent", body: "saved while group pending" },
      },
    ]);
    const saved = await store.get("drafts", "independent");
    store.close();
    return { denied, saved };
  });
  expect(result.denied).toBe(true);
  expect(result.saved.body).toBe("saved while group pending");
  await page.evaluate(async () => {
    (window as any).release();
    await (window as any).owned;
  });
  const status = await other.evaluate(async () => {
    const path = "/src/bulk_journal.ts",
      { BulkJournal } = await import(path);
    return BulkJournal.own(
      "M".repeat(43),
      async (j: any) => (await j.get("group")).counts,
    );
  });
  expect(status).toMatchObject({ done: 1, uncertain: 0 });
});

test("failed pages roll back counts and identities; only definite failures retry", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const path = "/src/bulk_journal.ts",
      { BulkJournal } = await import(path);
    return BulkJournal.own("N".repeat(43), async (j: any) => {
      const original = {
        id: "m",
        account: "work",
        folder: "INBOX",
        remoteId: "1.1",
        unread: true,
        starred: false,
      };
      const row = { position: 0, id: "m", account: "work", original };
      async function* duplicate() {
        yield [row, { ...row, position: 1 }];
      }
      let failed = false;
      try {
        await j.prepare(
          "duplicate",
          { kind: "flags", unread: false },
          2,
          duplicate(),
        );
      } catch {
        failed = true;
      }
      async function* one() {
        yield [row];
      }
      const review = await j.prepare(
        "retry",
        { kind: "flags", unread: false },
        1,
        one(),
      );
      await j.decide("retry", review.revision, "approve");
      const first = await j.claim("retry");
      const failure = await j.settle("retry", 0, first.attempt, {
        kind: "rejected",
        error: "Server rejected this flag change. Retry.",
      });
      await j.retry("retry", failure.revision, 0);
      const second = await j.claim("retry");
      let stale = false;
      try {
        await j.settle("retry", 0, first.attempt, {
          kind: "rejected",
          error: "old request",
        });
      } catch {
        stale = true;
      }
      const unknown = await j.settle("retry", 0, second.attempt, {
        kind: "uncertain",
        error: "Connection closed before acknowledgment. Check server mail.",
      });
      let refused = false;
      try {
        await j.retry("retry", unknown.revision, 0);
      } catch {
        refused = true;
      }
      return {
        failed,
        duplicate: await j.get("duplicate"),
        rows: await j.page("duplicate"),
        stale,
        refused,
        unknown,
        first: first.attempt,
        second: second.attempt,
      };
    });
  });
  expect(result.failed && result.stale && result.refused).toBe(true);
  expect(result.duplicate).toMatchObject({ staged: 0, counts: { pending: 0 } });
  expect(result.rows).toEqual([]);
  expect(result.first).not.toBe(result.second);
  expect(result.unknown).toMatchObject({
    paused: true,
    counts: { uncertain: 1, failed: 0, running: 0 },
  });
});

test("history is paged and profiles keep distinct journals", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const result = await page.evaluate(async () => {
    const path = "/src/bulk_journal.ts",
      { BulkJournal } = await import(path);
    const history = await BulkJournal.own("O".repeat(43), async (j: any) => {
      for (let i = 0; i < 23; i++) {
        async function* rows() {
          yield [{ position: 0, id: "m", account: "work", original: null }];
        }
        await j.prepare(
          `job-${i}`,
          { kind: "flags", unread: false },
          1,
          rows(),
        );
      }
      const first = await j.history(),
        last = first.at(-1),
        second = await j.history([last.created, last.id]);
      return { first, second };
    });
    return {
      ...history,
      other: await BulkJournal.own("P".repeat(43), async (j: any) =>
        j.history(),
      ),
    };
  });
  expect(result.first).toHaveLength(20);
  expect(result.second).toHaveLength(3);
  expect(
    new Set([...result.first, ...result.second].map((j: any) => j.id)).size,
  ).toBe(23);
  expect(result.other).toEqual([]);
});
