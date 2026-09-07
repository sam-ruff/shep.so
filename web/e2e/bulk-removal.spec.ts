import { test, expect, type Page } from "@playwright/test";
const profile = "R".repeat(43);
async function setup(page: Page, seed = true) {
  await page.route("**/api/session", (r) =>
    r.fulfill({
      json: {
        user_id: profile,
        email: "owner@example.test",
        csrf: "X".repeat(43),
      },
    }),
  );
  await page.route("**/removal-fixture", (r) =>
    r.fulfill({
      contentType: "text/html",
      body: "<!doctype html><title>Group removal fixture</title>",
    }),
  );
  await page.goto("/removal-fixture");
  await page.evaluate(
    async ({ profile, seed }) => {
      const storage = "/src/storage.ts",
        provider = "/src/provider.ts",
        bulk = "/src/bulk_journal.ts",
        executor = "/src/bulk_executor.ts";
      const { BrowserStore } = await import(storage),
        { GatewayRepository } = await import(provider),
        { BulkJournal } = await import(bulk),
        { BulkExecutor } = await import(executor);
      const store = await BrowserStore.open(profile);
      const accounts = ["work", "personal"].map((id) => ({
        id,
        name: id,
        email: `${id}@example.test`,
        protocol: "Pop3",
        host: "mail.example.test",
        port: 995,
        username: id,
        incoming_security: "Tls",
        incoming_auth: "Password",
        smtp_host: "mail.example.test",
        smtp_port: 465,
        smtp_username: id,
        smtp_security: "Tls",
        smtp_auth: "Automatic",
        smtp_separate_password: false,
        sent_copy: "LocalOnly",
        sent_folder: "Sent",
      }));
      const rows: any[] = [];
      if (seed) {
        const changes: any[] = accounts.map((value) => ({
          store: "accounts",
          key: value.id,
          value,
        }));
        for (let i = 0; i < 128; i++) {
          const account = i < 125 ? "work" : "personal",
            id = `m${String(i).padStart(3, "0")}`;
          const core = {
            id,
            account_id: account,
            remote_id: `42.${i + 1}`,
            folder: "INBOX",
            sender: "Sender <sender@example.test>",
            recipient: `${account}@example.test`,
            subject: `${account} letter ${i}`,
            preview: "Fictional letter",
            timestamp: 1000 - i,
            unread: true,
            starred: false,
            attachment_count: 0,
          };
          changes.push({
            store: "mail",
            key: id,
            value: { core, text: "Fictional cached body" },
          });
          rows.push({
            position: i,
            id,
            account,
            original: {
              id,
              account,
              remoteId: core.remote_id,
              folder: "INBOX",
              unread: true,
              starred: false,
            },
          });
        }
        await store.commit(changes);
      }
      const env: any = { store, BulkJournal, profile, rows, calls: [] };
      env.repo = new GatewayRepository(
        { user_id: profile, email: "owner@example.test", csrf: "X".repeat(43) },
        store,
        async (path: string) => {
          env.calls.push(path);
          throw Error(
            "No provider calls are permitted by this removal fixture",
          );
        },
      );
      await env.repo.load();
      env.executor = new BulkExecutor(profile, env.repo);
      env.stage = (
        id = "mixed",
        action = { kind: "flags", starred: true },
        selected = rows,
      ) =>
        BulkJournal.own(profile, (journal: any) =>
          journal.prepare(
            id,
            action,
            selected.length,
            (async function* () {
              for (let i = 0; i < selected.length; i += 50)
                yield selected.slice(i, i + 50);
            })(),
          ),
        );
      env.inspect = (fn: any) => BulkJournal.inspect(profile, fn);
      (window as any).removalFixture = env;
    },
    { profile, seed },
  );
}

test("group removal rejects stale reviews and unfinished decisions, removes only owned entries and fences stale captures", async ({
  page,
}) => {
  await setup(page);
  const r = await page.evaluate(async () => {
    const e = (window as any).removalFixture;
    await e.stage();
    const initial = await e.repo.removalPreview("work");
    let denied = "",
      stale = "";
    try {
      await e.repo.removeAccount(initial, false);
    } catch (error) {
      denied = String(error);
    }
    const job = await e.inspect((j: any) => j.get("mixed"));
    await e.executor.decide("mixed", job.revision, "approve");
    try {
      await e.repo.removeAccount(initial, true);
    } catch (error) {
      stale = String(error);
    }
    const current = await e.repo.removalPreview("work");
    await e.repo.removeAccount(current, true);
    const left = await e.inspect(async (j: any) => ({
      job: await j.get("mixed"),
      page: await j.page("mixed"),
      review: await j.accountReview("work"),
    }));
    let captured = "";
    try {
      await e.stage("stale-capture");
    } catch (error) {
      captured = String(error);
    }
    const run = await e.executor.run();
    const applied = (await e.store.all("mail")).filter(
      (m: any) => m.core.starred,
    ).length;
    return {
      initial: initial.groups,
      denied,
      stale,
      left,
      captured,
      run,
      applied,
      accounts: e.repo.accounts.map((a: any) => a.id),
      calls: e.calls,
    };
  });
  expect(r.initial).toMatchObject({ items: 125, jobs: 1, unfinished: 125 });
  expect(r.denied).toContain("Confirm discarding");
  expect(r.stale).toContain("Group work changed");
  expect(r.left.job).toMatchObject({
    total: 3,
    staged: 3,
    counts: { pending: 3 },
    pendingCache: 0,
  });
  expect(r.left.page.map((i: any) => i.account)).toEqual([
    "personal",
    "personal",
    "personal",
  ]);
  expect(r.left.review.items).toBe(0);
  expect(r.captured).toContain("removed");
  expect(r.run.steps).toBe(3);
  expect(r.applied).toBe(3);
  expect(r.accounts).toEqual(["personal"]);
  expect(r.calls).toEqual([]);
});

test("a failed mail transaction keeps every group record and a lost commit reply completes removal idempotently", async ({
  page,
}) => {
  await setup(page);
  const r = await page.evaluate(async () => {
    const e = (window as any).removalFixture;
    await e.stage();
    const review = await e.repo.removalPreview("work"),
      remove = e.store.removeAccount.bind(e.store);
    e.store.removeAccount = async () => {
      throw Error("Synthetic precommit storage failure");
    };
    let failed = "";
    try {
      await e.repo.removeAccount(review, true);
    } catch (error) {
      failed = String(error);
    }
    const before = await e.inspect((j: any) => j.get("mixed"));
    const present = !!(await e.store.get("accounts", "work"));
    e.store.removeAccount = async (...args: any[]) => {
      await remove(...args);
      throw Error("Synthetic lost committed reply");
    };
    await e.repo.removeAccount(review, true);
    await e.repo.removeAccount(review, true);
    return {
      failed,
      before,
      present,
      after: await e.inspect((j: any) => j.get("mixed")),
      warning: e.repo.warning,
      accounts: e.repo.accounts.map((a: any) => a.id),
    };
  });
  expect(r.failed).toContain("precommit");
  expect(r.before.total).toBe(128);
  expect(r.present).toBe(true);
  expect(r.after.total).toBe(3);
  expect(r.warning).toBeNull();
  expect(r.accounts).toEqual(["personal"]);
});

test("bounded cleanup rolls back a failed page and a fresh owner finishes before any queued provider step", async ({
  page,
}) => {
  await setup(page);
  const r = await page.evaluate(async () => {
    const e = (window as any).removalFixture;
    await e.stage();
    const job = await e.inspect((j: any) => j.get("mixed"));
    await e.executor.decide(job.id, job.revision, "approve");
    const review = await e.repo.removalPreview("work"),
      del = IDBObjectStore.prototype.delete,
      read = IDBIndex.prototype.getAll;
    let deletions = 0,
      maxPage = 0;
    IDBIndex.prototype.getAll = function (...args: any[]) {
      if (this.name === "owners") {
        maxPage = Math.max(maxPage, args[1] ?? Infinity);
      }
      return read.apply(this, args as any);
    };
    IDBObjectStore.prototype.delete = function (...args: any[]) {
      if (this.name === "items" && ++deletions === 60)
        throw Error("Synthetic second-page abort");
      return del.apply(this, args as any);
    };
    try {
      await e.repo.removeAccount(review, true);
    } finally {
      IDBObjectStore.prototype.delete = del;
      IDBIndex.prototype.getAll = read;
    }
    const warning = e.repo.warning,
      remaining = await e.inspect((j: any) => j.get("mixed"));
    const removed = await e.store.get("removedAccounts", "work");
    await e.repo.load();
    const cleaned = await e.inspect((j: any) => j.get("mixed"));
    const run = await e.executor.run();
    return {
      warning,
      remaining,
      removed,
      maxPage,
      cleaned,
      run,
      calls: e.calls,
      accounts: e.repo.accounts.map((a: any) => a.id),
    };
  });
  expect(r.warning).toContain("Account removed");
  expect(r.warning).toContain("cleanup is pending");
  expect(r.remaining.total).toBe(78); // exactly the first committed page was removed
  expect(r.maxPage).toBe(50);
  expect(r.cleaned.total).toBe(3);
  expect(r.run.steps).toBe(3);
  expect(r.calls).toEqual([]);
  expect(r.accounts).toEqual(["personal"]);
  expect(JSON.stringify(r.removed)).not.toContain("Fictional");
});

test("another tab cannot remove an account while a group owns a running step", async ({
  page,
  context,
}) => {
  await setup(page);
  await page.evaluate(async () => {
    const e = (window as any).removalFixture;
    const j = await e.stage();
    await e.executor.decide(j.id, j.revision, "approve");
    e.held = e.BulkJournal.own(e.profile, async (journal: any) => {
      e.item = await journal.claim("mixed");
      await new Promise<void>((r) => {
        e.release = r;
      });
      await journal.settle("mixed", e.item.position, e.item.attempt, {
        kind: "rejected",
        error: "Fictional definite rejection",
      });
    });
  });
  await expect
    .poll(() => page.evaluate(() => !!(window as any).removalFixture.release))
    .toBe(true);
  const other = await context.newPage();
  await setup(other, false);
  const r = await other.evaluate(async () => {
    const e = (window as any).removalFixture,
      review = await e.repo.removalPreview("work");
    let refused = "";
    try {
      await e.repo.removeAccount(review, true);
    } catch (error) {
      refused = String(error);
    }
    const job = await e.inspect((j: any) => j.get("mixed"));
    return { refused, job, present: !!(await e.store.get("accounts", "work")) };
  });
  expect(r.refused).toContain("active in another tab");
  expect(r.job.counts.running).toBe(1);
  expect(r.job.counts.uncertain).toBe(0);
  expect(r.present).toBe(true);
  await page.evaluate(async () => {
    const e = (window as any).removalFixture;
    e.release();
    await e.held;
  });
  await other.close();
});

test("tab loss after the mail commit resumes cleanup before the next owner can claim", async ({
  page,
  context,
}) => {
  await setup(page);
  await page.evaluate(async () => {
    const e = (window as any).removalFixture;
    const job = await e.stage();
    await e.executor.decide(job.id, job.revision, "approve");
    const review = await e.repo.removalPreview("work"),
      reconcile = e.BulkJournal.prototype.reconcileAccounts;
    let count = 0;
    e.BulkJournal.prototype.reconcileAccounts = async function () {
      if (++count === 2) {
        e.cleaning = true;
        await new Promise(() => {});
      }
      return reconcile.call(this);
    };
    e.pending = e.repo.removeAccount(review, true);
  });
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const e = (window as any).removalFixture;
        return e.cleaning && !!(await e.store.get("removedAccounts", "work"));
      }),
    )
    .toBe(true);
  await page.close();
  const fresh = await context.newPage();
  await fresh.goto("/preview.html");
  await expect
    .poll(() =>
      fresh.evaluate(
        async (profile) =>
          !(await navigator.locks.query()).held?.some(
            (l) => l.name === `shep.bulk.v1.${profile}`,
          ),
        profile,
      ),
    )
    .toBe(true);
  await setup(fresh, false);
  const r = await fresh.evaluate(async () => {
    const e = (window as any).removalFixture;
    return {
      job: await e.inspect((j: any) => j.get("mixed")),
      run: await e.executor.run(),
      calls: e.calls,
      accounts: e.repo.accounts.map((a: any) => a.id),
    };
  });
  expect(r.job).toMatchObject({
    total: 3,
    counts: { pending: 3, uncertain: 0 },
  });
  expect(r.run.steps).toBe(3);
  expect(r.accounts).toEqual(["personal"]);
  expect(r.calls).toEqual([]);
  await fresh.close();
});

test("completed, uncertain, inverse and missing group entries are reviewed and only the removed ownership is deleted", async ({
  page,
}) => {
  await setup(page);
  const r = await page.evaluate(async () => {
    const e = (window as any).removalFixture;
    await e.BulkJournal.own(e.profile, async (j: any) => {
      const rows = e.rows.slice(0, 4);
      rows[3] = { ...rows[3], original: null };
      rows.push(e.rows[125]);
      let job = await j.prepare(
        "history",
        { kind: "flags", starred: true },
        rows.length,
        (async function* () {
          yield rows;
        })(),
      );
      await j.decide(job.id, job.revision, "approve", 1);
      for (let i = 0; i < 2; i++) {
        const item = await j.claim(job.id);
        await j.settle(job.id, item.position, item.attempt, {
          kind: "committed",
          receipt: {
            before: item.original,
            after: { ...item.original, starred: true },
          },
          cacheApplied: true,
        });
      }
      const unknown = await j.claim(job.id);
      job = await j.settle(job.id, unknown.position, unknown.attempt, {
        kind: "uncertain",
        error: "Fictional unknown outcome",
      });
      job = await j.decide(job.id, job.revision, "undo", 2);
      job = await j.decide(job.id, job.revision, "resume");
      const inverse = await j.claim(job.id);
      await j.settle(job.id, inverse.position, inverse.attempt, {
        kind: "committed",
        receipt: {
          before: inverse.receipt.after,
          after: inverse.receipt.before,
        },
        cacheApplied: false,
      });
    });
    const review = await e.repo.removalPreview("work");
    await e.repo.removeAccount(review, true);
    return {
      review: review.groups,
      left: await e.inspect(async (j: any) => ({
        job: await j.get("history"),
        page: await j.page("history"),
        repairs: await j.pendingCache(),
      })),
    };
  });
  expect(r.review).toMatchObject({ items: 4, jobs: 1, unfinished: 3 });
  expect(r.left.job).toMatchObject({
    total: 1,
    staged: 1,
    undo: true,
    pendingCache: 0,
    counts: { pending: 1, restored: 0, done: 0, missing: 0, uncertain: 0 },
  });
  expect(r.left.page[0].account).toBe("personal");
  expect(r.left.repairs).toEqual([]);
});

for (const theme of ["light", "dark"] as const)
  test(`Preferences ${theme} review shows group counts, requires discard and rejects changed review through real controls`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 900, height: 640 });
    await page.emulateMedia({ colorScheme: theme });
    await setup(page);
    await page.evaluate(async () => {
      await (window as any).removalFixture.stage();
    });
    await page.goto("/");
    await page
      .getByRole("button", { name: "Preferences", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Remove work@example.test", exact: true })
      .click();
    const dialog = page.getByRole("dialog", {
      name: "Remove work@example.test",
    });
    await expect(dialog).toContainText("125 group entries");
    await expect(dialog).toContainText("1 related group");
    await expect(dialog).toContainText("125 unfinished group changes");
    const remove = dialog.getByRole("button", {
      name: "Remove from browser",
      exact: true,
    });
    await expect(remove).toBeDisabled();
    await dialog
      .getByRole("checkbox", {
        name: "Discard unfinished delivery and mail-change records",
      })
      .check();
    await page.evaluate(async (profile) => {
      const path = "/src/bulk_journal.ts",
        { BulkJournal } = await import(path);
      await BulkJournal.own(profile, async (j: any) => {
        const job = await j.get("mixed");
        await j.decide(job.id, job.revision, "approve", 1);
      });
    }, profile);
    await remove.click();
    await expect(dialog).toContainText("Group work changed");
    await dialog
      .getByRole("button", { name: "Reload removal counts", exact: true })
      .click();
    await expect(dialog).toContainText("125 unfinished group changes");
    await expect(remove).toBeDisabled();
    await dialog
      .getByRole("checkbox", {
        name: "Discard unfinished delivery and mail-change records",
      })
      .check();
    await page.screenshot({
      path: `../artifacts/web/group-removal-${theme}.png`,
    });
    await remove.click();
    await expect(dialog).toHaveCount(0);
    await expect(
      page.getByRole("button", {
        name: "Remove personal@example.test",
        exact: true,
      }),
    ).toBeVisible();
    const state = await page.evaluate(async (profile) => {
      const path = "/src/bulk_journal.ts",
        { BulkJournal } = await import(path);
      return BulkJournal.inspect(profile, (j: any) => j.get("mixed"));
    }, profile);
    expect(state.total).toBe(3);
  });

test("schema upgrade waits for the old receipt owner and indexes both source and destination account history", async ({
  page,
}) => {
  await setup(page);
  const r = await page.evaluate(async () => {
    const e = (window as any).removalFixture,
      profile = "M".repeat(43),
      path = "/src/storage.ts",
      { BrowserStore } = await import(path);
    const store = await BrowserStore.open(profile);
    await store.commit([
      {
        store: "accounts",
        key: "personal",
        value: { id: "personal", email: "personal@example.test" },
      },
    ]);
    const old = await new Promise<IDBDatabase>((resolve, reject) => {
      const r = indexedDB.open(`shep.bulk.v1.${profile}`, 3);
      r.onupgradeneeded = () => {
        const jobs = r.result.createObjectStore("jobs", { keyPath: "id" }),
          items = r.result.createObjectStore("items", {
            keyPath: ["job", "position"],
          });
        jobs.createIndex("created", ["created", "id"]);
        jobs.createIndex("state", "state");
        jobs.createIndex("queue", ["runnable", "created", "id"]);
        items.createIndex("status", ["job", "status", "position"]);
        items.createIndex("recovery", "status");
        items.createIndex("identity", ["job", "id"], { unique: true });
        items.createIndex("cache", ["cache", "job", "position"]);
        for (const id of ["cross", "keep"]) {
          const original = {
            id: `mail-${id}`,
            account: "work",
            remoteId: "42.1",
            folder: "INBOX",
            starred: false,
            unread: true,
          };
          jobs.put({
            id,
            action:
              id === "cross"
                ? { kind: "move", folder: "Archive", account: "personal" }
                : { kind: "flags", starred: true },
            state: "ready",
            created: 1,
            revision: 3,
            total: 1,
            staged: 1,
            lastPosition: 0,
            paused: false,
            undo: false,
            pendingCache: 0,
            forwardIntent: 1,
            counts: {
              pending: 0,
              running: 0,
              done: 1,
              undo_running: 0,
              restored: 0,
              failed: 0,
              uncertain: 0,
              missing: 0,
              skipped: 0,
            },
          });
          items.put({
            job: id,
            position: 0,
            id: original.id,
            account: "work",
            original,
            phase: "forward",
            status: "done",
            cache: 0,
            attempt: "original-attempt",
            receipt: {
              before: original,
              after:
                id === "cross"
                  ? {
                      ...original,
                      account: "personal",
                      folder: "Archive",
                      remoteId: "91.9",
                    }
                  : { ...original, starred: true },
            },
          });
        }
      };
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error);
    });
    let upgraded = false;
    old.onversionchange = () => {
      upgraded = true;
      old.close();
    };
    let refusal = "",
      wrote = false;
    await navigator.locks.request(`shep.bulk.v1.${profile}`, async () => {
      try {
        await e.BulkJournal.inspect(profile, (j: any) => j.history());
      } catch (error) {
        refusal = String(error);
      }
      if (upgraded) throw Error("The live receipt owner was closed");
      await new Promise<void>((resolve, reject) => {
        const tx = old.transaction("jobs", "readwrite");
        tx.objectStore("jobs").get("keep");
        tx.oncomplete = () => {
          wrote = true;
          resolve();
        };
        tx.onabort = () => reject(tx.error);
      });
    });
    const review = await e.BulkJournal.inspect(profile, (j: any) =>
      j.accountReview("personal"),
    );
    const removalPath = "/src/account_removal.ts",
      { removalPreview, reviewStores } = await import(removalPath);
    await e.BulkJournal.own(profile, async (j: any) => {
      await store.removeAccount(
        removalPreview(await store.snapshot(reviewStores), "personal"),
        false,
      );
      await j.reconcileAccounts();
    });
    const left = await e.BulkJournal.inspect(profile, async (j: any) => ({
      history: await j.history(),
      page: await j.page("keep"),
      personal: await j.accountReview("personal"),
    }));
    store.close();
    return { upgraded, refusal, wrote, review, left };
  });
  expect(r.refusal).toContain("Finish group work");
  expect(r.wrote && r.upgraded).toBe(true);
  expect(r.review).toMatchObject({ items: 1, jobs: 1, unfinished: 0 });
  expect(r.left.history.map((j: any) => j.id)).toEqual(["keep"]);
  expect(r.left.page[0].receipt.after).toMatchObject({
    account: "work",
    starred: true,
    remoteId: "42.1",
  });
  expect(r.left.personal.items).toBe(0);
});

test("a committed removal shows its cleanup warning through real controls and reopening finishes it", async ({
  page,
}) => {
  await setup(page);
  await page.evaluate(async () => {
    await (window as any).removalFixture.stage();
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page
    .getByRole("button", { name: "Remove work@example.test", exact: true })
    .click();
  const dialog = page.getByRole("dialog", { name: "Remove work@example.test" });
  await expect(dialog).toContainText("125 unfinished group changes");
  await dialog
    .getByRole("checkbox", {
      name: "Discard unfinished delivery and mail-change records",
    })
    .check();
  await page.evaluate(() => {
    const del = IDBObjectStore.prototype.delete;
    IDBObjectStore.prototype.delete = function (...args: any[]) {
      if (this.name === "items")
        throw Error("Synthetic unavailable history storage");
      return del.apply(this, args as any);
    };
  });
  await dialog
    .getByRole("button", { name: "Remove from browser", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("alert")).toContainText(
    "Account removed. Group history cleanup is pending",
  );
  await expect(
    page.getByRole("button", { name: "Remove work@example.test", exact: true }),
  ).toHaveCount(0);
  await page.screenshot({
    path: "../artifacts/web/group-removal-cleanup-warning.png",
  });
  await page.reload();
  await expect(
    page.getByRole("button", { name: "Preferences", exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  const state = await page.evaluate(async (profile) => {
    const path = "/src/bulk_journal.ts",
      { BulkJournal } = await import(path);
    return BulkJournal.inspect(profile, (j: any) => j.get("mixed"));
  }, profile);
  expect(state.total).toBe(3);
});
