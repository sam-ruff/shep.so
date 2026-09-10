import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { seed, profile, subject } from "./mailbox-fixture";

test.afterEach(async ({ page }, info) => {
  if (info.status === info.expectedStatus) return;
  const observed = await page
    .evaluate(async (profile) => {
      const journalPath = "/src/bulk_journal.ts",
        storagePath = "/src/storage.ts";
      const { BulkJournal } = await import(journalPath),
        { BrowserStore } = await import(storagePath);
      const store = await BrowserStore.open(profile);
      try {
        return await BulkJournal.inspect(profile, async (j: any) => ({
          history: await j.history(),
          gaps: await j.pendingCache(),
          mail: await store.get("mail", "m000"),
          metadata: await store.get("mailMetadata", "m000"),
          text: document.body.innerText,
        }));
      } finally {
        store.close();
      }
    }, profile)
    .catch((error) => ({ error: String(error) }));
  await info.attach("synthetic-group-observation", {
    body: JSON.stringify(observed, null, 2),
    contentType: "application/json",
  });
});

async function selectAll(page: Page) {
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page
    .getByRole("button", { name: "Select all messages", exact: true })
    .click();
  await expect(
    page.getByRole("button", {
      name: "Archive selected messages",
      exact: true,
    }),
  ).toBeEnabled();
}
async function history(page: Page) {
  await page
    .getByRole("button", { name: "Group history", exact: true })
    .click();
  const d = page.getByRole("dialog", { name: "Group history", exact: true });
  await d
    .getByRole("button", { name: /^Archive 125 messages/ })
    .first()
    .click();
  return d;
}
async function cached(page: Page) {
  return page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    const rows = await store.all("mail");
    store.close();
    return rows.reduce((r: Record<string, number>, m: any) => {
      r[m.core.folder] = (r[m.core.folder] ?? 0) + 1;
      return r;
    }, {});
  }, profile);
}

test("actual mixed-account group review cancels safely, applies all 125 rows and restores them through bounded History", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 920 });
  await seed(page);
  await selectAll(page);
  await page
    .getByRole("button", { name: "Archive selected messages", exact: true })
    .click();
  const review = page.getByRole("dialog", {
    name: "Review group action",
    exact: true,
  });
  await expect(review).toContainText("125 messages selected across all pages");
  await expect(review).toContainText("work@example.test");
  await expect(review).toContainText("personal@example.test");
  await page.screenshot({ path: "../artifacts/web/bulk-review-light.png" });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await review
    .getByRole("button", { name: "Cancel group action", exact: true })
    .click();
  expect(await cached(page)).toEqual({ INBOX: 125 });
  await page
    .getByRole("button", { name: "Archive selected messages", exact: true })
    .click();
  await review
    .getByRole("button", { name: "Archive 125 messages", exact: true })
    .click();
  await expect(page.locator("main > header")).toContainText(
    "0 messages · 0 unread",
  );
  await expect.poll(() => cached(page)).toEqual({ Archive: 125 });
  const d = await history(page);
  await expect(d.locator(".group-progress")).toContainText("125 changed");
  await expect(d.locator(".group-item")).toHaveCount(50);
  await d.getByRole("button", { name: "Next results", exact: true }).click();
  await expect(d.locator(".group-item").first()).toContainText("Message 51");
  await d.getByRole("button", { name: "Next results", exact: true }).click();
  await expect(d.locator(".group-item")).toHaveCount(25);
  await d.getByRole("button", { name: "Undo group", exact: true }).click();
  await expect(d.locator(".group-progress")).toContainText("125 restored");
  expect(await cached(page)).toEqual({ INBOX: 125 });
  await page.screenshot({ path: "../artifacts/web/bulk-history-light.png" });
});

test("actual group Undo stays clickable through a held provider receipt and cancels unsent work", async ({
  page,
}) => {
  await page.setViewportSize({ width: 900, height: 640 });
  await seed(page);
  await page.evaluate(async () => {
    const path = "/src/provider.ts",
      { GatewayRepository } = await import(path),
      mutate = GatewayRepository.prototype.mutateWithReceipt;
    const calls: string[] = [];
    Object.assign(window, { groupCalls: calls });
    let hold = true;
    GatewayRepository.prototype.mutateWithReceipt = async function (
      ...args: any[]
    ) {
      calls.push(args[0]);
      if (hold) {
        hold = false;
        await new Promise<void>((resolve) =>
          Object.assign(window, { releaseGroup: resolve }),
        );
      }
      return mutate.apply(this, args);
    };
  });
  await selectAll(page);
  await page
    .getByRole("button", { name: "Archive selected messages", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Review group action", exact: true })
    .getByRole("button", { name: "Archive 125 messages", exact: true })
    .click();
  await expect
    .poll(() => page.evaluate(() => (window as any).groupCalls.length))
    .toBe(1);
  await expect(page.locator("main > header")).toContainText(
    "0 messages · 0 unread",
  );
  expect(await cached(page)).toEqual({ INBOX: 125 });
  const notice = page.getByRole("status", {
    name: "Group notification",
    exact: true,
  });
  await notice.getByRole("button", { name: "Undo group", exact: true }).click();
  await expect(notice).toContainText("Undo requested for 125 messages");
  await expect(page.locator("main > header")).toContainText(
    "125 messages · 125 unread",
  );
  await expect(
    page.getByRole("button", { name: subject(0), exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: "../artifacts/web/bulk-pending-undo-compact.png",
  });
  await page.evaluate(() => (window as any).releaseGroup());
  const d = await history(page);
  await expect(d.locator(".group-progress")).toContainText("1 restored");
  await expect(d.locator(".group-progress")).toContainText(
    "124 cancelled before sending",
  );
  expect(await cached(page)).toEqual({ INBOX: 125 });
  expect(await page.evaluate(() => (window as any).groupCalls.length)).toBe(2);
});

test("failed and unconfirmed results remain visible; History retries only a definite failure and requires an explicit folder review", async ({
  page,
}) => {
  await seed(page);
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page
    .getByRole("combobox", { name: "Theme", exact: true })
    .selectOption("dark");
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await page.evaluate(async () => {
    const path = "/src/provider.ts",
      modelPath = "/src/model.ts";
    const { GatewayRepository } = await import(path),
      { MutationFailure } = await import(modelPath),
      mutate = GatewayRepository.prototype.mutateWithReceipt;
    const calls: string[] = [];
    Object.assign(window, { groupCalls: calls });
    GatewayRepository.prototype.mutateWithReceipt = async function (
      ...args: any[]
    ) {
      calls.push(args[0]);
      if (
        args[0] === "m000" &&
        calls.filter((id) => id === "m000").length === 1
      )
        throw Error("Synthetic server rejection. Review and retry.");
      if (args[0] === "m001")
        throw new MutationFailure(
          "Synthetic lost provider reply. Check server folders.",
        );
      return mutate.apply(this, args);
    };
  });
  await selectAll(page);
  await page
    .getByRole("button", { name: "Archive selected messages", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Review group action", exact: true })
    .getByRole("button", { name: "Archive 125 messages", exact: true })
    .click();
  await expect(
    page.getByRole("alert", { name: "Saved group actions", exact: true }),
  ).toContainText("1 failed");
  const d = await history(page);
  await expect(d.locator(".group-progress")).toContainText("1 unconfirmed");
  const accept = d.getByRole("button", {
    name: "Accept current state for message 2",
    exact: true,
  });
  await expect(accept).toBeDisabled();
  await expect(d.locator(".group-item").first()).toContainText(subject(0));
  await page.screenshot({ path: "../artifacts/web/bulk-recovery-dark.png" });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await d
    .getByRole("checkbox", {
      name: "I checked server folders for message 2",
      exact: true,
    })
    .check();
  await accept.click();
  await expect(d.locator(".group-progress")).toContainText("0 unconfirmed");
  await d.getByRole("button", { name: "Retry message 1", exact: true }).click();
  const resume = d.getByRole("button", { name: "Resume group", exact: true });
  if (await resume.isVisible()) await resume.click();
  await expect(d.locator(".group-progress")).toContainText("124 changed");
  await expect(d.locator(".group-progress")).toContainText(
    "1 unavailable or skipped",
  );
  expect(
    await page.evaluate(
      () =>
        (window as any).groupCalls.filter((id: string) => id === "m001").length,
    ),
  ).toBe(1);
  expect(await cached(page)).toEqual({ INBOX: 1, Archive: 124 });
  expect(
    await page.evaluate(async (profile) => {
      const path = "/src/storage.ts",
        { BrowserStore } = await import(path),
        store = await BrowserStore.open(profile);
      const intent = await store.get("mailIntents", "m001");
      store.close();
      return intent.fields.folder.status;
    }, profile),
  ).toBe("failed");
});

test("a native held Undo press survives receipt progress, then restores only dispatched messages", async ({
  page,
}) => {
  await seed(page);
  await page.evaluate(async () => {
    const path = "/src/provider.ts",
      { GatewayRepository } = await import(path),
      mutate = GatewayRepository.prototype.mutateWithReceipt;
    const calls: string[] = [],
      releases: (() => void)[] = [];
    Object.assign(window, { groupCalls: calls, groupReleases: releases });
    GatewayRepository.prototype.mutateWithReceipt = async function (
      ...args: any[]
    ) {
      calls.push(args[0]);
      if (calls.length <= 2)
        await new Promise<void>((resolve) => releases.push(resolve));
      return mutate.apply(this, args);
    };
  });
  await selectAll(page);
  await page
    .getByRole("button", { name: "Archive selected messages", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Review group action", exact: true })
    .getByRole("button", { name: "Archive 125 messages", exact: true })
    .click();
  await expect
    .poll(() => page.evaluate(() => (window as any).groupCalls.length))
    .toBe(1);
  const notice = page.getByRole("status", {
    name: "Group notification",
    exact: true,
  });
  const undo = notice.getByRole("button", { name: "Undo group", exact: true });
  const original = await undo.elementHandle(),
    box = await undo.boundingBox();
  await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
  await page.mouse.down();
  await page.evaluate(() => (window as any).groupReleases[0]());
  await expect
    .poll(() => page.evaluate(() => (window as any).groupCalls.length))
    .toBe(2);
  expect(await original!.evaluate((n) => n.isConnected)).toBe(true);
  await page.mouse.up();
  await expect(notice).toContainText("Undo requested for 125 messages");
  await page.evaluate(() => (window as any).groupReleases[1]());
  const d = await history(page);
  await expect(d.locator(".group-progress")).toContainText("2 restored");
  await expect(d.locator(".group-progress")).toContainText(
    "123 cancelled before sending",
  );
  expect(await cached(page)).toEqual({ INBOX: 125 });
  expect(await page.evaluate(() => (window as any).groupCalls.length)).toBe(4);
});

test("startup exposes an abandoned provider step for review and never automatically repeats it", async ({
  page,
}) => {
  await seed(page);
  await page.goto("/seed-selection");
  await page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      journalPath = "/src/bulk_journal.ts";
    const { BrowserStore } = await import(path),
      { BulkJournal } = await import(journalPath),
      store = await BrowserStore.open(profile);
    const state = await store.get("cacheState", "mail");
    const records = await Promise.all([
      store.get("mail", "m000"),
      store.get("mail", "m001"),
    ]);
    await BulkJournal.own(profile, async (j: any) => {
      async function* chunks() {
        yield records.map((m: any, position: number) => ({
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
      }
      let job = await j.prepare(
        "abandoned",
        { kind: "move", folder: "Archive", account: null },
        2,
        chunks(),
        state.epoch,
      );
      job = await j.decideCurrent(
        job,
        "approve",
        await store.intents.reserve(),
      );
      await j.claim(job.id);
    });
    store.close();
  }, profile);
  await page.goto("/");
  await page
    .getByRole("button", { name: "Group history", exact: true })
    .click();
  const d = page.getByRole("dialog", { name: "Group history", exact: true });
  await d.getByRole("button", { name: /^Archive 2 messages/ }).click();
  await expect(d.locator(".group-progress")).toContainText("1 unconfirmed");
  await expect(d.locator(".group-progress")).toContainText("Paused");
  expect(await cached(page)).toEqual({ INBOX: 125 });
  await d
    .getByRole("checkbox", {
      name: "I checked server folders for message 1",
      exact: true,
    })
    .check();
  await d
    .getByRole("button", {
      name: "Accept current state for message 1",
      exact: true,
    })
    .click();
  await expect(d.locator(".group-progress")).toContainText("0 unconfirmed");
  await d.getByRole("button", { name: "Resume group", exact: true }).click();
  await expect(d.locator(".group-progress")).toContainText("1 changed");
  expect(await cached(page)).toEqual({ INBOX: 124, Archive: 1 });
});

test("a group captured during an earlier move flags the same full membership at its acknowledged destination", async ({
  page,
}) => {
  await seed(page);
  await page.evaluate(async () => {
    const path = "/src/provider.ts",
      { GatewayRepository } = await import(path),
      mutate = GatewayRepository.prototype.mutateWithReceipt;
    const calls: string[] = [];
    Object.assign(window, { groupCalls: calls });
    let hold = true;
    GatewayRepository.prototype.mutateWithReceipt = async function (
      ...args: any[]
    ) {
      calls.push(`${args[0]}:${args[1].folder ?? "flags"}`);
      if (calls.length === 6)
        await new Promise<void>((resolve) =>
          Object.assign(window, { releaseHistoryProgress: resolve }),
        );
      if (hold) {
        hold = false;
        await new Promise<void>((resolve) =>
          Object.assign(window, { releaseGroup: resolve }),
        );
      }
      return mutate.apply(this, args);
    };
  });
  await selectAll(page);
  await page
    .getByRole("button", { name: "Archive selected messages", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Review group action", exact: true })
    .getByRole("button", { name: "Archive 125 messages", exact: true })
    .click();
  await expect
    .poll(() => page.evaluate(() => (window as any).groupCalls.length))
    .toBe(1);
  await page
    .getByRole("navigation", { name: "Workspace", exact: true })
    .getByRole("button", { name: "Archive", exact: true })
    .click();
  await expect(page.locator("main > header")).toContainText("125 messages");
  await selectAll(page);
  await page
    .getByRole("button", { name: "Flag selected messages", exact: true })
    .click();
  const review = page.getByRole("dialog", {
    name: "Review group action",
    exact: true,
  });
  await expect(review).toContainText("Preparing");
  await page.evaluate(() => (window as any).releaseGroup());
  await review
    .getByRole("button", { name: "Flag 125 messages", exact: true })
    .click();
  await page.evaluate(async () => {
    const path = "/src/mailbox_worker_client.ts",
      groupsPath = "/src/bulk_client.ts",
      { MailboxWorkerClient } = await import(path),
      { BrowserGroups } = await import(groupsPath),
      query = MailboxWorkerClient.prototype.page;
    const view = BrowserGroups.prototype.view;
    let first = true;
    BrowserGroups.prototype.view = async function (...args: any[]) {
      if (first) {
        first = false;
        await new Promise<void>((resolve) =>
          Object.assign(window, { releaseInitialHistory: resolve }),
        );
      }
      return view.apply(this, args);
    };
    const pending = new Promise<void>((resolve) =>
      Object.assign(window, { releaseHistoryPreview: resolve }),
    );
    MailboxWorkerClient.prototype.page = async function (...args: any[]) {
      if (args[0].previewOnly) {
        Object.assign(window, { historyPreviewPending: true });
        await pending;
      }
      return query.apply(this, args);
    };
  });
  await page
    .getByRole("button", { name: "Group history", exact: true })
    .click();
  const d = page.getByRole("dialog", { name: "Group history", exact: true });
  // Observe each queued job through the same five-second per-job assertion.
  await d.getByRole("button", { name: /^Archive 125 messages/ }).click();
  await expect
    .poll(() =>
      page.evaluate(() => typeof (window as any).releaseInitialHistory),
    )
    .toBe("function");
  await expect
    .poll(() =>
      page.evaluate(() => typeof (window as any).releaseHistoryProgress),
    )
    .toBe("function");
  await page.evaluate(() => (window as any).releaseHistoryProgress());
  await expect
    .poll(() => page.evaluate(() => (window as any).historyPreviewPending))
    .toBe(true);
  await expect(d.locator(".group-progress")).toContainText("125 changed");
  await expect(
    d.getByRole("button", { name: "Undo group", exact: true }),
  ).toBeDisabled();
  await page.screenshot({
    path: "../artifacts/web/bulk-history-pending-preview.png",
  });
  await page.evaluate(() => (window as any).releaseHistoryPreview());
  await page.evaluate(() => (window as any).releaseInitialHistory());
  await expect(
    d.getByRole("button", { name: "Undo group", exact: true }),
  ).toBeEnabled();
  await page.evaluate(async () => {
    const path = "/src/bulk_client.ts",
      { BrowserGroups } = await import(path),
      view = BrowserGroups.prototype.view;
    let first = true;
    BrowserGroups.prototype.view = async function (...args: any[]) {
      if (first) {
        first = false;
        await new Promise<void>((resolve) =>
          Object.assign(window, { releaseHistory: resolve }),
        );
      }
      return view.apply(this, args);
    };
  });
  await d.getByRole("button", { name: /^Flag 125 messages/ }).click();
  await expect(d.locator(".group-detail")).toBeHidden();
  await expect
    .poll(() => page.evaluate(() => typeof (window as any).releaseHistory))
    .toBe("function");
  await page.evaluate(() => (window as any).releaseHistory());
  await expect(
    d.getByRole("heading", { name: "Flag · 125 messages", exact: true }),
  ).toBeVisible();
  await expect(d.locator(".group-progress")).toContainText("125 changed");
  await expect(d.locator(".group-progress")).toContainText("0 failed");
  const final = await page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    const rows = await store.all("mail");
    store.close();
    return {
      count: rows.length,
      correct: rows.filter(
        (m: any) => m.core.folder === "Archive" && m.core.starred,
      ).length,
    };
  }, profile);
  expect(final).toEqual({ count: 125, correct: 125 });
  await page.screenshot({
    path: "../artifacts/web/bulk-successive-groups.png",
  });
});

test("group Undo restores rows and counts while its decision and subsequent query are both held", async ({
  page,
}) => {
  await seed(page);
  await selectAll(page);
  await page
    .getByRole("button", { name: "Archive selected messages", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Review group action", exact: true })
    .getByRole("button", { name: "Archive 125 messages", exact: true })
    .click();
  await expect.poll(() => cached(page)).toEqual({ Archive: 125 });
  await expect(page.locator("main > header")).toContainText(
    "0 messages · 0 unread",
  );
  await page.evaluate(async () => {
    const path = "/src/bulk_journal.ts",
      workerPath = "/src/mailbox_worker_client.ts";
    const { BulkJournal } = await import(path),
      { MailboxWorkerClient } = await import(workerPath);
    const decide = BulkJournal.prototype.decideCurrent,
      query = MailboxWorkerClient.prototype.page;
    let held = false;
    BulkJournal.prototype.decideCurrent = async function (...args: any[]) {
      if (args[1] === "undo") {
        held = true;
        await new Promise<void>((resolve) =>
          Object.assign(window, { releaseUndoDecision: resolve }),
        );
      }
      return decide.apply(this, args);
    };
    const queries = new Promise<void>((resolve) =>
      Object.assign(window, { releaseUndoQueries: resolve }),
    );
    MailboxWorkerClient.prototype.page = async function (...args: any[]) {
      if (held) await queries;
      return query.apply(this, args);
    };
  });
  const notice = page.getByRole("status", {
    name: "Group notification",
    exact: true,
  });
  await notice.getByRole("button", { name: "Undo group", exact: true }).click();
  await expect(notice).toContainText("Undo requested for 125 messages");
  await expect(page.locator("main > header")).toContainText(
    "125 messages · 125 unread",
  );
  await expect(page.locator(".mail-row")).toHaveCount(50);
  await expect(
    page.getByRole("button", { name: subject(0), exact: true }),
  ).toBeVisible();
  expect(await cached(page)).toEqual({ Archive: 125 });
  await page.screenshot({
    path: "../artifacts/web/bulk-undo-before-storage.png",
  });
  await page.evaluate(() => {
    (window as any).releaseUndoDecision();
    (window as any).releaseUndoQueries();
  });
  await expect.poll(() => cached(page)).toEqual({ INBOX: 125 });
  await expect(page.locator("main > header")).toContainText(
    "125 messages · 125 unread",
  );
});

test("History Undo paints before saving, retains newer flag intent and reports rejection after History closes", async ({
  page,
}) => {
  await seed(page);
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.locator(".mail-row").first().getByRole("checkbox").check();
  await page
    .getByRole("button", { name: "Archive selected messages", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Review group action", exact: true })
    .getByRole("button", { name: "Archive 1 message", exact: true })
    .click();
  await expect.poll(() => cached(page)).toEqual({ Archive: 1, INBOX: 124 });
  await page
    .getByRole("button", { name: "Group history", exact: true })
    .click();
  const d = page.getByRole("dialog", { name: "Group history", exact: true });
  await d.getByRole("button", { name: /^Archive 1 message/ }).click();
  await expect(d.locator(".group-progress")).toContainText("1 changed");
  await page.evaluate(async () => {
    const path = "/src/bulk_journal.ts",
      { BulkJournal } = await import(path),
      decide = BulkJournal.prototype.decideCurrent;
    BulkJournal.prototype.decideCurrent = async function (...args: any[]) {
      if (args[1] === "undo") {
        await new Promise<void>((_resolve, reject) =>
          Object.assign(window, {
            rejectUndo: () =>
              reject(Error("Synthetic Undo storage refusal. Retry Undo.")),
          }),
        );
      }
      return decide.apply(this, args);
    };
  });
  await d.getByRole("button", { name: "Undo group", exact: true }).click();
  await expect(page.locator("main > header")).toContainText(
    "125 messages · 125 unread",
  );
  await d.getByRole("button", { name: "Close", exact: true }).click();
  const row = page.locator(".mail-row").filter({
    has: page.getByRole("button", { name: subject(0), exact: true }),
  });
  await row.getByRole("button", { name: subject(0), exact: true }).click();
  await row
    .getByRole("button", { name: `Flag ${subject(0)}`, exact: true })
    .click();
  await expect(
    row.getByRole("button", { name: `Unflag ${subject(0)}`, exact: true }),
  ).toBeVisible();
  await page.evaluate(() => (window as any).rejectUndo());
  await expect(
    page.getByText("Synthetic Undo storage refusal. Retry Undo.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.locator("main > header")).toContainText(
    "124 messages · 124 unread",
  );
  await expect(
    page
      .getByRole("region", { name: "Message reader" })
      .getByRole("button", { name: "Unflag", exact: true }),
  ).toBeEnabled();
  await expect
    .poll(() =>
      page.evaluate(async (profile) => {
        const path = "/src/storage.ts",
          { BrowserStore } = await import(path),
          store = await BrowserStore.open(profile);
        const mail = await store.get("mail", "m000");
        store.close();
        return { folder: mail.core.folder, starred: mail.core.starred };
      }, profile),
    )
    .toEqual({ folder: "Archive", starred: true });
  await page.screenshot({
    path: "../artifacts/web/bulk-undo-storage-failure.png",
  });
});

test("a failed History Undo preview leaves progress usable and retries through Refresh", async ({
  page,
}) => {
  await seed(page);
  await selectAll(page);
  await page
    .getByRole("button", { name: "Archive selected messages", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Review group action", exact: true })
    .getByRole("button", { name: "Archive 125 messages", exact: true })
    .click();
  await expect.poll(() => cached(page)).toEqual({ Archive: 125 });
  await page.evaluate(async () => {
    const path = "/src/mailbox_worker_client.ts",
      { MailboxWorkerClient } = await import(path),
      query = MailboxWorkerClient.prototype.page;
    Object.assign(window, { rejectHistoryPreview: true });
    MailboxWorkerClient.prototype.page = async function (...args: any[]) {
      if (args[0].previewOnly && (window as any).rejectHistoryPreview)
        throw Error("The saved Undo preview could not be loaded.");
      return query.apply(this, args);
    };
  });
  const d = await history(page);
  await expect(d.locator(".group-progress")).toContainText("125 changed");
  const error = d
    .getByRole("alert")
    .filter({ hasText: "Refresh history to retry Undo" });
  await expect(error).toBeVisible();
  await expect(
    d.getByRole("button", { name: "Undo group", exact: true }),
  ).toBeDisabled();
  await expect(
    d.getByRole("button", { name: "Pause group", exact: true }),
  ).toBeEnabled();
  await page.screenshot({
    path: "../artifacts/web/bulk-history-preview-retry.png",
  });
  await page.evaluate(() =>
    Object.assign(window, { rejectHistoryPreview: false }),
  );
  await d.getByRole("button", { name: "Refresh history", exact: true }).click();
  await expect(error).toBeHidden();
  await expect(
    d.getByRole("button", { name: "Undo group", exact: true }),
  ).toBeEnabled();
  await d.getByRole("button", { name: "Undo group", exact: true }).click();
  await expect(d.locator(".group-progress")).toContainText("125 restored");
  expect(await cached(page)).toEqual({ INBOX: 125 });
});
