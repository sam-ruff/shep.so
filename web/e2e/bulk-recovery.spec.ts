import { expect, test, type BrowserContext, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { seed, profile, subject } from "./mailbox-fixture";

async function boot(page: Page, setup: () => Promise<void>) {
  let start!: () => void;
  const pending = new Promise<void>((resolve) => {
    start = resolve;
  });
  await page.route("**/api/session", async (route) => {
    await pending;
    await route.fulfill({
      json: {
        email: "owner@example.test",
        user_id: profile,
        csrf: "X".repeat(43),
      },
    });
  });
  await page.goto("/", { waitUntil: "commit" });
  try {
    await page.evaluate(setup);
  } finally {
    start();
  }
}

async function savedRecovery(page: Page) {
  await seed(page);
  await page.goto("/seed-selection");
  await page.evaluate(async (profile) => {
    const storagePath = "/src/storage.ts",
      journalPath = "/src/bulk_journal.ts",
      { BrowserStore } = await import(storagePath),
      { BulkJournal } = await import(journalPath),
      store = await BrowserStore.open(profile);
    const state = await store.get("cacheState", "mail");
    const records = await Promise.all(
      Array.from({ length: 58 }, (_, i) =>
        store.get("mail", `m${i.toString().padStart(3, "0")}`),
      ),
    );
    const originals = (start: number, count: number) =>
      records.slice(start, start + count).map((m: any, position: number) => ({
        id: m.core.id,
        account: m.core.account_id,
        position,
        original: {
          id: m.core.id,
          account: m.core.account_id,
          folder: m.core.folder,
          remoteId: m.core.remote_id,
          unread: m.core.unread,
          starred: m.core.starred,
        },
      }));
    await BulkJournal.own(profile, async (j: any) => {
      async function create(
        id: string,
        start: number,
        count: number,
        move = false,
      ) {
        async function* chunks() {
          const rows = originals(start, count);
          for (let i = 0; i < rows.length; i += 50) yield rows.slice(i, i + 50);
        }
        return j.prepare(
          id,
          move
            ? { kind: "move", folder: "Archive", account: null }
            : { kind: "flags", starred: true },
          count,
          chunks(),
          state.epoch,
        );
      }
      let failed = await create("older-failed", 0, 52);
      failed = await j.decideCurrent(
        failed,
        "approve",
        await store.intents.reserve(),
      );
      for (let i = 0; i < 52; i++) {
        const item = await j.claim(failed.id);
        await j.settle(failed.id, item.position, item.attempt, {
          kind: "rejected",
          error: "Synthetic server rejection. Reconnect before retrying.",
        });
      }
      let uncertain = await create("older-uncertain", 52, 2, true);
      uncertain = await j.decideCurrent(
        uncertain,
        "approve",
        await store.intents.reserve(),
      );
      await j.claim(uncertain.id);
      async function* interrupted() {
        yield originals(54, 1);
        throw Error("Synthetic shutdown during staging");
      }
      await j
        .prepare(
          "older-interrupted",
          { kind: "flags", starred: true },
          2,
          interrupted(),
          state.epoch,
        )
        .catch(() => {});
      // A frozen review whose tab is gone: 60 staged rows need two bounded
      // cleanup transactions at the next owner's startup.
      await create("older-abandoned", 0, 58);
      await j.prepare(
        "older-abandoned-tail",
        { kind: "flags", starred: true },
        2,
        (async function* () {
          yield originals(0, 2);
        })(),
        state.epoch,
      );
      // Approved but paused groups keep the older entries beyond the first
      // History page without ever becoming runnable.
      for (let i = 0; i < 25; i++) {
        const paused = await create(`new-paused-${i}`, 57, 1);
        await j.decideCurrent(
          await j.decideCurrent(
            paused,
            "approve",
            await store.intents.reserve(),
          ),
          "pause",
        );
      }
      let busy = await create("new-queued", 56, 1);
      busy = await j.decideCurrent(
        busy,
        "approve",
        await store.intents.reserve(),
      );
      const recent = await j.history();
      if (
        recent.length !== 20 ||
        recent.some((job: any) => job.id.startsWith("older-"))
      )
        throw Error("Recovery fixtures must be outside the first History page");
    });
    store.close();
  }, profile);

  await boot(page, async () => {
    const path = "/src/provider.ts",
      { GatewayRepository } = await import(path),
      mutate = GatewayRepository.prototype.mutateWithReceipt;
    const calls: string[] = [];
    Object.assign(window, { recoveryCalls: calls });
    GatewayRepository.prototype.mutateWithReceipt = async function (
      ...args: any[]
    ) {
      calls.push(args[0]);
      await new Promise<void>((resolve) =>
        Object.assign(window, { releaseQueuedGroup: resolve }),
      );
      return mutate.apply(this, args);
    };
  });
  await expect(
    page.getByRole("button", { name: subject(0), exact: true }),
  ).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => typeof (window as any).releaseQueuedGroup))
    .toBe("function");
}

/** Which seeded abandoned reviews still exist, and how many of their rows. */
function retiredReviews(page: Page) {
  return page.evaluate(async (profile) => {
    const path = "/src/bulk_journal.ts",
      { BulkJournal } = await import(path);
    return BulkJournal.inspect(profile, async (j: any) => {
      const present = (id: string) =>
        j.get(id).then(
          () => true,
          () => false,
        );
      let rows = 0;
      for (const id of [
        "older-abandoned",
        "older-abandoned-tail",
        "older-interrupted",
      ])
        rows += (await j.page(id)).length;
      return {
        abandoned: await present("older-abandoned"),
        tail: await present("older-abandoned-tail"),
        interrupted: await present("older-interrupted"),
        rows,
      };
    });
  }, profile);
}

for (const theme of ["light", "dark"] as const) {
  test(`startup ${theme} finds older failed and unconfirmed groups and retires abandoned reviews while queued work is pending`, async ({
    page,
  }) => {
    await page.setViewportSize(
      theme === "dark"
        ? { width: 900, height: 640 }
        : { width: 1440, height: 920 },
    );
    await savedRecovery(page);
    if (theme === "dark") {
      await page.route("**/api/capabilities", (route) =>
        route.fulfill({ json: { mail: false, endpoints: [] } }),
      );
      await page
        .getByRole("button", { name: "Preferences", exact: true })
        .click();
      await page.getByLabel("Theme", { exact: true }).selectOption("dark");
      await page.getByRole("button", { name: "Mail", exact: true }).click();
    }
    const banner = page.getByRole("alert", {
      name: "Saved group actions",
      exact: true,
    });
    await expect(banner).toContainText("52 failed");
    expect(
      (await new AxeBuilder({ page }).include(".group-recovery").analyze())
        .violations,
    ).toEqual([]);
    await page.screenshot({
      path: `../artifacts/web/bulk-startup-summary-${theme}.png`,
    });
    await banner
      .getByRole("button", { name: "Review saved group actions", exact: true })
      .click();
    await expect(banner).toContainText("52 changes failed");
    await expect(banner).toContainText("1 change has an unconfirmed result");
    // Abandoned reviews and interrupted staging were retired by the new
    // owner before its held provider step, so nothing is left to review.
    await expect(banner).not.toContainText("review ended before approval");
    await expect
      .poll(() => retiredReviews(page))
      .toEqual({
        abandoned: false,
        tail: false,
        interrupted: false,
        rows: 0,
      });
    expect(
      (await new AxeBuilder({ page }).include(".group-recovery").analyze())
        .violations,
    ).toEqual([]);
    expect(await page.evaluate(() => (window as any).recoveryCalls)).toEqual([
      "m056",
    ]);
    await page.screenshot({
      path: `../artifacts/web/bulk-startup-recovery-${theme}.png`,
    });
    await page.evaluate(async () => {
      const path = "/src/bulk_journal.ts",
        { BulkJournal } = await import(path),
        attention = BulkJournal.prototype.attention;
      Object.assign(window, { rejectRecoveryCheck: true });
      BulkJournal.prototype.attention = function (...args: any[]) {
        if ((window as any).rejectRecoveryCheck)
          return Promise.reject(Error("Synthetic observation failure"));
        return attention.apply(this, args);
      };
    });
    await banner
      .getByRole("button", { name: "Refresh saved group status", exact: true })
      .click();
    await expect(banner).toContainText("Could not check saved group actions");
    await expect(banner).toContainText("52 changes failed");
    await expect(
      banner.getByRole("button", {
        name: "Review unconfirmed group changes",
        exact: true,
      }),
    ).toBeEnabled();
    await page.evaluate(() =>
      Object.assign(window, { rejectRecoveryCheck: false }),
    );
    await banner
      .getByRole("button", { name: "Refresh saved group status", exact: true })
      .click();
    await expect(banner).not.toContainText(
      "Could not check saved group actions",
    );
    await banner
      .getByRole("button", {
        name: "Review unconfirmed group changes",
        exact: true,
      })
      .click();
    const history = page.getByRole("dialog", {
      name: "Group history",
      exact: true,
    });
    await expect(
      history.getByRole("heading", {
        name: "Archive · 2 messages",
        exact: true,
      }),
    ).toBeVisible();
    await expect(history.locator(".group-progress")).toContainText(
      "1 unconfirmed",
    );
    await expect(history.locator(".group-progress")).toContainText("Paused");
    await expect(
      history.getByRole("button", {
        name: "Accept current state for message 1",
        exact: true,
      }),
    ).toBeDisabled();
    await history
      .getByRole("checkbox", {
        name: "I checked server folders for message 1",
        exact: true,
      })
      .check();
    await history
      .getByRole("button", {
        name: "Accept current state for message 1",
        exact: true,
      })
      .click();
    await expect(history.locator(".group-progress")).toContainText(
      "0 unconfirmed",
    );
    await history.getByRole("button", { name: "Close", exact: true }).click();
    await expect(
      banner.getByRole("button", {
        name: "Review unconfirmed group changes",
        exact: true,
      }),
    ).toBeHidden();
    await banner
      .getByRole("button", { name: "Review failed group changes", exact: true })
      .click();
    await expect(
      history.getByRole("heading", { name: "Flag · 52 messages", exact: true }),
    ).toBeVisible();
    await expect(history.locator(".group-progress")).toContainText("52 failed");
    await expect(history.locator(".group-item")).toHaveCount(50);
    await history
      .getByRole("button", { name: "Next results", exact: true })
      .click();
    await expect(history.locator(".group-item")).toHaveCount(2);
    await history.getByRole("button", { name: "Close", exact: true }).click();
    await page.evaluate(() => (window as any).releaseQueuedGroup());
    await expect
      .poll(() =>
        page.evaluate(async (profile) => {
          const path = "/src/bulk_journal.ts",
            { BulkJournal } = await import(path);
          return BulkJournal.inspect(
            profile,
            async (j: any) => (await j.get("new-queued")).counts.done,
          );
        }, profile),
      )
      .toBe(1);
    expect(await page.evaluate(() => (window as any).recoveryCalls)).toEqual([
      "m056",
    ]);
  });
}

async function archiveAll(page: Page) {
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page
    .getByRole("button", { name: "Select all messages", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Archive selected messages", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Review group action", exact: true })
    .getByRole("button", { name: "Archive 125 messages", exact: true })
    .click();
}

async function failCacheRepair() {
  const path = "/src/provider.ts",
    { GatewayRepository } = await import(path),
    repair = GatewayRepository.prototype.repairMutation,
    mutate = GatewayRepository.prototype.mutateWithReceipt;
  const calls: string[] = [];
  Object.assign(window, { recoveryCalls: calls, rejectCacheRepair: true });
  GatewayRepository.prototype.repairMutation = async function (...args: any[]) {
    if ((window as any).rejectCacheRepair)
      throw Error(
        "The confirmed result could not finish saving locally. Retry from History.",
      );
    return repair.apply(this, args);
  };
  GatewayRepository.prototype.mutateWithReceipt = async function (
    ...args: any[]
  ) {
    calls.push(args[0]);
    return mutate.apply(this, args);
  };
}

test("startup points to saved receipts and cache retry never repeats the acknowledged mail operation", async ({
  page,
}) => {
  await seed(page);
  await page.evaluate(failCacheRepair);
  await archiveAll(page);
  const banner = page.getByRole("alert", {
    name: "Saved group actions",
    exact: true,
  });
  await expect(banner).toContainText("1 awaiting local repair");
  await banner
    .getByRole("button", { name: "Review saved group actions", exact: true })
    .click();
  await expect(banner).toContainText("1 confirmed change needs local repair");
  expect(await page.evaluate(() => (window as any).recoveryCalls)).toEqual([
    "m000",
  ]);
  await boot(page, failCacheRepair);
  await expect(banner).toContainText("1 awaiting local repair");
  await banner
    .getByRole("button", { name: "Review saved group actions", exact: true })
    .click();
  await expect(banner).toContainText("1 confirmed change needs local repair");
  expect(await page.evaluate(() => (window as any).recoveryCalls)).toEqual([]);
  await page.screenshot({
    path: "../artifacts/web/bulk-startup-cache-repair.png",
  });
  await banner
    .getByRole("button", { name: "Review saved group results", exact: true })
    .click();
  const history = page.getByRole("dialog", {
    name: "Group history",
    exact: true,
  });
  await expect(history.locator(".group-progress")).toContainText("1 changed");
  await expect(history.locator(".group-progress")).toContainText("124 waiting");
  await page.evaluate(() =>
    Object.assign(window, { rejectCacheRepair: false }),
  );
  await history
    .getByRole("button", { name: "Retry cached results", exact: true })
    .click();
  await expect(history.locator(".group-progress")).toContainText("125 changed");
  await history.getByRole("button", { name: "Close", exact: true }).click();
  await expect(banner).toBeHidden();
  const calls = await page.evaluate(
    () => (window as any).recoveryCalls as string[],
  );
  expect(calls).toHaveLength(124);
  expect(calls).not.toContain("m000");
});

async function reviewJobs(page: Page) {
  return page.evaluate(async (profile) => {
    const path = "/src/bulk_journal.ts",
      { BulkJournal } = await import(path);
    return BulkJournal.inspect(profile, async (j: any) =>
      Promise.all(
        (await j.history()).map(async (job: any) => ({
          state: job.state,
          rows: (await j.page(job.id)).length,
        })),
      ),
    );
  }, profile);
}
async function openSecondTab(context: BrowserContext) {
  const other = await context.newPage();
  await other.route("**/api/session", (route) =>
    route.fulfill({
      json: {
        email: "owner@example.test",
        user_id: profile,
        csrf: "X".repeat(43),
      },
    }),
  );
  await other.goto("/");
  await expect(
    other.getByRole("button", { name: subject(0), exact: true }),
  ).toBeVisible();
  return other;
}

test("a review open in a live tab survives another owner's sweep; the same review is retired once its tab closes", async ({
  page,
  context,
}) => {
  await seed(page);
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page
    .getByRole("button", { name: "Select all messages", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Archive selected messages", exact: true })
    .click();
  const review = page.getByRole("dialog", {
    name: "Review group action",
    exact: true,
  });
  await expect(
    review.getByRole("button", { name: "Archive 125 messages", exact: true }),
  ).toBeFocused();
  const other = await openSecondTab(context);
  // The second owner has started and swept; the live review keeps its rows.
  await other
    .getByRole("button", { name: "Group history", exact: true })
    .click();
  const history = other.getByRole("dialog", {
    name: "Group history",
    exact: true,
  });
  await expect(
    history.getByRole("button", { name: /^Archive 125 messages/ }),
  ).toBeVisible();
  await history.getByRole("button", { name: "Close", exact: true }).click();
  // Wait for the second owner's startup run (and its sweep) to finish.
  await expect
    .poll(() =>
      other.evaluate(
        async (profile) =>
          (await navigator.locks.query()).held?.some(
            (lock) => lock.name === `shep.bulk.v1.${profile}`,
          ) ?? false,
        profile,
      ),
    )
    .toBe(false);
  expect(await reviewJobs(other)).toEqual([{ state: "review", rows: 50 }]);
  await expect(
    review.getByRole("button", { name: "Archive 125 messages", exact: true }),
  ).toBeEnabled();
  // Both tabs hold their own review liveness lock; only the reviewing tab's
  // lock disappears when it closes.
  const tabLocks = (target: Page) =>
    target.evaluate(
      async (profile) =>
        (await navigator.locks.query()).held?.filter((lock) =>
          lock.name?.startsWith(`shep.bulk.tab.${profile}.`),
        ).length ?? 0,
      profile,
    );
  expect(await tabLocks(other)).toBe(2);
  await page.close();
  await expect.poll(() => tabLocks(other)).toBe(1);
  await other.reload();
  await expect(
    other.getByRole("button", { name: subject(0), exact: true }),
  ).toBeVisible();
  await expect.poll(() => reviewJobs(other)).toEqual([]);
  await other
    .getByRole("button", { name: "Group history", exact: true })
    .click();
  await expect(history).toContainText("No group changes on this device.");
  await expect(
    other.getByRole("alert", { name: "Saved group actions", exact: true }),
  ).toBeHidden();
  const folders = await other.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    try {
      return (await store.all("mail")).map((mail: any) => mail.core.folder);
    } finally {
      store.close();
    }
  }, profile);
  expect(folders).toEqual(Array(125).fill("INBOX"));
  await other.close();
});

test("a second tab observes live ownership without recovery and reports uncertainty only after the owner closes", async ({
  page,
  context,
}) => {
  await seed(page);
  await page.evaluate(async () => {
    const path = "/src/provider.ts",
      { GatewayRepository } = await import(path),
      mutate = GatewayRepository.prototype.mutateWithReceipt;
    GatewayRepository.prototype.mutateWithReceipt = async function (
      ...args: any[]
    ) {
      await new Promise<void>((resolve) =>
        Object.assign(window, { heldOwnerStep: resolve }),
      );
      return mutate.apply(this, args);
    };
  });
  await archiveAll(page);
  await expect
    .poll(() => page.evaluate(() => typeof (window as any).heldOwnerStep))
    .toBe("function");
  const other = await context.newPage();
  await other.route("**/api/session", (route) =>
    route.fulfill({
      json: {
        email: "owner@example.test",
        user_id: profile,
        csrf: "X".repeat(43),
      },
    }),
  );
  await other.goto("/");
  await other
    .getByRole("button", { name: "Group history", exact: true })
    .click();
  const history = other.getByRole("dialog", {
    name: "Group history",
    exact: true,
  });
  await history.getByRole("button", { name: /^Archive 125 messages/ }).click();
  await expect(history.locator(".group-progress")).toContainText(
    "1 in progress",
  );
  await expect(history.locator(".group-progress")).toContainText(
    "0 unconfirmed",
  );
  const attention = await other.evaluate(async (profile) => {
    const path = "/src/bulk_journal.ts",
      { BulkJournal } = await import(path);
    return BulkJournal.inspect(profile, (j: any) => j.attention());
  }, profile);
  expect(attention).toEqual([]);
  await page.close();
  await expect
    .poll(() =>
      other.evaluate(
        async (profile) =>
          (await navigator.locks.query()).held?.some(
            (lock) => lock.name === `shep.bulk.v1.${profile}`,
          ) ?? false,
        profile,
      ),
    )
    .toBe(false);
  await other.reload();
  const banner = other.getByRole("alert", {
    name: "Saved group actions",
    exact: true,
  });
  await expect(banner).toContainText("1 unconfirmed");
  await banner
    .getByRole("button", { name: "Review saved group actions", exact: true })
    .click();
  await expect(banner).toContainText("1 change has an unconfirmed result");
  await banner
    .getByRole("button", {
      name: "Review unconfirmed group changes",
      exact: true,
    })
    .click();
  await expect(history.locator(".group-progress")).toContainText(
    "1 unconfirmed",
  );
  await expect(history.locator(".group-progress")).toContainText("Paused");
  const folders = await other.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    try {
      return (await store.all("mail")).map((mail: any) => mail.core.folder);
    } finally {
      store.close();
    }
  }, profile);
  expect(folders).toEqual(Array(125).fill("INBOX"));
  await other.close();
});
