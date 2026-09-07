import { test, expect, type Page } from "@playwright/test";

const profile = "I".repeat(43);
async function seed(page: Page) {
  await page.route("**/api/session", (r) =>
    r.fulfill({
      json: {
        email: "owner@example.test",
        user_id: profile,
        csrf: "X".repeat(43),
      },
    }),
  );
  await page.route("**/seed-intents", (r) =>
    r.fulfill({
      contentType: "text/html",
      body: "<!doctype html><title>Mail intent fixture</title>",
    }),
  );
  await page.goto("/seed-intents");
  await page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    const changes: any[] = [];
    for (const id of ["work", "personal"]) {
      changes.push({
        store: "accounts",
        key: id,
        value: {
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
        },
      });
      const core = {
        id,
        account_id: id,
        remote_id: "42.7",
        folder: "INBOX",
        sender: "Sender <sender@example.test>",
        recipient: `${id}@example.test`,
        subject: `${id} intent letter`,
        preview: "Fictional cached letter",
        timestamp: 100,
        unread: true,
        starred: false,
        attachment_count: 0,
      };
      changes.push({
        store: "mail",
        key: id,
        value: { core, text: "Fictional cached body" },
      });
    }
    await store.commit(changes);
    store.close();
  }, profile);
}

test("tab decisions keep field ownership through identical values, stale completions and group Undo", async ({
  page,
  context,
}) => {
  await seed(page);
  const other = await context.newPage();
  await other.goto("/seed-intents");
  const first = await page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    const lease = await store.intents.register("work", {
      starred: true,
      unread: false,
    });
    await store.intents.finish(lease, "applied");
    store.close();
    return lease;
  }, profile);
  const newer = await other.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    const lease = await store.intents.register("work", { starred: true });
    store.close();
    return lease;
  }, profile);
  expect(newer.revision).toBeGreaterThan(first.revision);
  const result = await page.evaluate(
    async ({ profile, first, newer }) => {
      const path = "/src/storage.ts",
        { BrowserStore } = await import(path),
        store = await BrowserStore.open(profile);
      // A group approved before the later tab choice cannot claim that field.
      const stale = await store.intents.claim("work", first.revision, {
        starred: true,
      });
      const inverse = await store.intents.reserve();
      const undo = await store.intents.claim(
        "work",
        inverse,
        { starred: false, unread: true },
        first.revision,
      );
      await store.intents.finish(first, "failed");
      await store.intents.finish(newer, "applied");
      await store.intents.finish(newer, "failed");
      const effective = await store.intents.effective(undo),
        saved = await store.get("mailIntents", "work");
      // Repeating an acknowledged revision is not another provider command.
      const repeat = await store.intents.claim("work", newer.revision, {
        starred: true,
      });
      const before = JSON.stringify(saved);
      let changed = false;
      try {
        await store.intents.claim("work", newer.revision, { starred: false });
      } catch {
        changed = true;
      }
      const unchanged =
        before === JSON.stringify(await store.get("mailIntents", "work"));
      store.close();
      return { stale, undo, effective, saved, repeat, changed, unchanged };
    },
    { profile, first, newer },
  );
  expect(result.stale.fields).toEqual({});
  expect(result.undo.fields).toEqual({ unread: true });
  expect(result.effective).toEqual({ unread: true });
  expect(result.saved.fields.starred).toMatchObject({
    revision: newer.revision,
    value: true,
    status: "applied",
  });
  expect(result.repeat.fields).toEqual({});
  expect(result.changed && result.unchanged).toBe(true);
  await other.close();
});

test("identity adoption merges intent atomically and follows aliases without erasing newer fields", async ({
  page,
}) => {
  await seed(page);
  const r = await page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    const mail = await store.get("mail", "work");
    await store.commit([
      {
        store: "mail",
        key: "duplicate",
        value: { ...mail, localId: "duplicate" },
      },
    ]);
    const older = await store.intents.register("work", {
      starred: true,
      unread: false,
    });
    const newer = await store.intents.register("duplicate", { starred: false });
    const before = await store.snapshot([
      "mail",
      "mailAliases",
      "mailIntents",
      "cacheState",
    ]);
    const changes = [
      {
        store: "mailAliases",
        key: "work",
        value: { alias: "work", target: "duplicate" },
      },
      { store: "mail", key: "work" },
    ];
    let aborted = false;
    try {
      await store.commit([
        ...changes,
        { store: "drafts", key: "bad", value: () => {} },
      ]);
    } catch {
      aborted = true;
    }
    const rollback =
      JSON.stringify(before) ===
      JSON.stringify(
        await store.snapshot([
          "mail",
          "mailAliases",
          "mailIntents",
          "cacheState",
        ]),
      );
    await store.commit(changes);
    let mismatch = false;
    try {
      await store.intents.effective(older, "personal");
    } catch {
      mismatch = true;
    }
    if (!mismatch)
      throw Error("Another message incorrectly accepted this lease");
    await store.intents.finish(older, "applied");
    const effective = await store.intents.effective(newer),
      target = await store.get("mailIntents", "duplicate"),
      old = await store.get("mailIntents", "work");
    // A subsequent ordinary sync never copies over field ownership.
    const current = await store.get("mail", "duplicate");
    current.core.starred = true;
    await store.commit([{ store: "mail", key: "duplicate", value: current }]);
    const unchanged =
      JSON.stringify(target) ===
      JSON.stringify(await store.get("mailIntents", "duplicate"));
    store.close();
    return { aborted, rollback, effective, target, old, unchanged };
  }, profile);
  expect(r.aborted && r.rollback && r.unchanged).toBe(true);
  expect(r.old).toBeUndefined();
  expect(r.effective).toEqual({ starred: false });
  expect(r.target.fields.unread.status).toBe("applied");
  expect(r.target.fields.starred.status).toBe("pending");
});

test("pending changes invalidate removal reviews and cannot recreate removed accounts", async ({
  page,
}) => {
  await seed(page);
  const r = await page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      removal = "/src/account_removal.ts",
      { BrowserStore } = await import(path),
      { removalPreview, reviewStores } = await import(removal),
      store = await BrowserStore.open(profile);
    const old = removalPreview(await store.snapshot(reviewStores), "work");
    const work = await store.intents.register("work", {
      starred: true,
      unread: false,
    });
    const personal = await store.intents.register("personal", {
      starred: true,
    });
    let stale = false,
      needsReview = false,
      late = false;
    try {
      await store.removeAccount(old, true);
    } catch {
      stale = true;
    }
    // Missing cache rows still have owned pending actions to review.
    await store.commit([{ store: "mail", key: "work" }]);
    const review = removalPreview(await store.snapshot(reviewStores), "work");
    try {
      await store.removeAccount(review, false);
    } catch {
      needsReview = true;
    }
    await store.removeAccount(review, true);
    await store.removeAccount(review, true);
    let rawRefused = false;
    try {
      await store.commit([
        { store: "raw", key: "work", value: "Late fictional body" },
      ]);
    } catch {
      rawRefused = true;
    }
    if (!rawRefused)
      throw Error("A removed missing-message identity accepted a late body");
    try {
      await store.commit([
        {
          store: "mailIntents",
          key: "work",
          value: { id: "work", account: "work", fields: {} },
        },
      ]);
    } catch {
      late = true;
    }
    const remaining = await store.all("mailIntents"),
      removed = await store.get("removedAccounts", "work");
    store.close();
    return {
      stale,
      needsReview,
      late,
      changes: review.changes,
      remaining,
      personal,
      removed,
    };
  }, profile);
  expect(r.stale && r.needsReview && r.late).toBe(true);
  expect(r.changes).toBe(1);
  expect(r.remaining).toHaveLength(1);
  expect(r.remaining[0].account).toBe("personal");
  expect(JSON.stringify(r.removed)).not.toContain("Fictional");
});

test("version-five upgrade retains metadata, drafts and Sent roles with a fresh monotonic clock", async ({
  page,
}) => {
  await seed(page);
  const r = await page.evaluate(async () => {
    const path = "/src/storage.ts",
      { BrowserStore, stores } = await import(path),
      profile = "V".repeat(43);
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const r = indexedDB.open(`shep.mail.v1.${profile}`, 5);
      r.onupgradeneeded = () => {
        for (const name of stores.filter(
          (n: string) => !["mailIntents", "intentState"].includes(n),
        ))
          r.result.createObjectStore(name);
        r.transaction!.objectStore("outgoing").createIndex("submission", "id");
        r.transaction!.objectStore("mailMetadata").createIndex(
          "newest",
          "newest",
        );
        r.transaction!.objectStore("mailMetadata").createIndex(
          "oldest",
          "oldest",
        );
      };
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error);
    });
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(
        ["mailRoles", "drafts", "cacheState"],
        "readwrite",
      );
      tx.objectStore("mailRoles").put(
        { account: "work", acknowledged: ["Actual Sent"] },
        "work",
      );
      tx.objectStore("drafts").put({ id: "d", body: "Keep this draft" }, "d");
      tx.objectStore("cacheState").put({ revision: 99, floor: 30 }, "mail");
      tx.oncomplete = () => resolve();
      tx.onabort = () => reject(tx.error);
    });
    db.close();
    const store = await BrowserStore.open(profile),
      snapshot = await store.snapshot([
        "drafts",
        "mailRoles",
        "cacheState",
        "mailIntents",
      ]);
    const first = await store.intents.reserve();
    store.close();
    const reopened = await BrowserStore.open(profile),
      second = await reopened.intents.reserve();
    await reopened.commit([
      { store: "intentState", key: "clock", value: Number.MAX_SAFE_INTEGER },
    ]);
    let overflow = false;
    try {
      await reopened.intents.reserve();
    } catch {
      overflow = true;
    }
    const clock = await reopened.get("intentState", "clock");
    reopened.close();
    return { snapshot, first, second, overflow, clock };
  });
  expect(r.snapshot.drafts).toEqual([{ id: "d", body: "Keep this draft" }]);
  expect(r.snapshot.mailRoles[0].acknowledged).toEqual(["Actual Sent"]);
  expect(r.snapshot.cacheState).toEqual([{ revision: 99, floor: 30 }]);
  expect(r.snapshot.mailIntents).toEqual([]);
  expect([r.first, r.second]).toEqual([1, 2]);
  expect(r.overflow).toBe(true);
  expect(r.clock).toBe(Number.MAX_SAFE_INTEGER);
});

test("row controls reserve at input time and skip an older queued flag while retaining the latest display", async ({
  page,
}) => {
  await seed(page);
  await page.goto("/");
  const flag = page.getByRole("button", {
    name: "Flag work intent letter",
    exact: true,
  });
  await expect(flag).toBeVisible();
  await page.evaluate(async () => {
    const path = "/src/provider.ts",
      { GatewayRepository } = await import(path),
      original = GatewayRepository.prototype.mutate;
    let release!: () => void;
    const held = new Promise<void>((r) => (release = r));
    (window as any).releaseIntent = release;
    let first = true;
    GatewayRepository.prototype.mutate = async function (...args: any[]) {
      if (first) {
        first = false;
        (window as any).intentHeld = true;
        await held;
      }
      return original.apply(this, args);
    };
  });
  await flag.click();
  await expect
    .poll(() => page.evaluate(() => (window as any).intentHeld))
    .toBe(true);
  await page
    .getByRole("button", { name: "Unflag work intent letter", exact: true })
    .click();
  await expect(flag).toBeVisible();
  const saved = await page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    const before = await store.get("mailIntents", "work");
    const group = await store.intents.reserve();
    const lease = await store.intents.claim("work", group, { starred: true });
    // Simulates an acknowledged newer owner while earlier UI work is held.
    const mail = await store.get("mail", "work");
    mail.core.starred = true;
    await store.commit([{ store: "mail", key: "work", value: mail }]);
    await store.intents.finish(lease, "applied");
    store.close();
    return { before, group };
  }, profile);
  expect(saved.before.fields.starred.value).toBe(false);
  expect(saved.before.fields.starred.revision).toBeLessThan(saved.group);
  await page.evaluate(() => (window as any).releaseIntent());
  await expect(
    page.getByRole("button", {
      name: "Unflag work intent letter",
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  const final = await page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    const mail = await store.get("mail", "work"),
      intent = await store.get("mailIntents", "work");
    store.close();
    return { mail, intent };
  }, profile);
  expect(final.mail.core.starred).toBe(true);
  expect(final.intent.fields.starred.revision).toBe(saved.group);
});

test("account removal controls review unfinished flag ownership and preserve the other account", async ({
  page,
}) => {
  await page.setViewportSize({ width: 900, height: 640 });
  await seed(page);
  await page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    await store.intents.register("work", { starred: true });
    store.close();
  }, profile);
  await page.goto("/");
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page
    .getByRole("button", { name: "Remove work@example.test", exact: true })
    .click();
  const dialog = page.getByRole("dialog", { name: "Remove work@example.test" });
  await expect(dialog).toContainText("1 unfinished mail change");
  await expect(
    dialog.getByRole("button", { name: "Remove from browser", exact: true }),
  ).toBeDisabled();
  await dialog
    .getByRole("checkbox", {
      name: "Discard unfinished delivery and mail-change records",
    })
    .check();
  await page.screenshot({ path: "../artifacts/web/intent-removal-review.png" });
  await dialog
    .getByRole("button", { name: "Remove from browser", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByRole("button", {
      name: "Remove personal@example.test",
      exact: true,
    }),
  ).toBeVisible();
});
