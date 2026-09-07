import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { seed, profile, subject } from "./mailbox-fixture";

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
    page.getByRole("alert", { name: "Group error", exact: true }),
  ).toContainText("group changes failed");
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
