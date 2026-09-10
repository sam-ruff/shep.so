import { test, expect, type Page } from "@playwright/test";
import { seed, subject } from "./mailbox-fixture";

async function holdBodies(page: Page) {
  await page.evaluate(async () => {
    const path = "/src/mailbox_worker_client.ts";
    const { MailboxWorkerClient } = await import(path);
    const detail = MailboxWorkerClient.prototype.detail;
    const calls: any[] = [];
    MailboxWorkerClient.prototype.detail = async function (id: string) {
      const result = await detail.call(this, id);
      return new Promise((resolve, reject) =>
        calls.push({ id, resolve: () => resolve(result), reject }),
      );
    };
    Object.assign(window, { bodyCalls: calls });
  });
}

test("actual body loading preserves a held row Flag press and its keyboard focus", async ({
  page,
}) => {
  await seed(page);
  await holdBodies(page);
  await page.getByRole("button", { name: subject(0), exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => (window as any).bodyCalls.length))
    .toBe(1);
  const flag = page.getByRole("button", {
    name: `Flag ${subject(1)}`,
    exact: true,
  });
  await flag.focus();
  const original = await flag.elementHandle();
  const box = await flag.boundingBox();
  await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
  await page.mouse.down();
  await page.evaluate(() => (window as any).bodyCalls[0].resolve());
  await expect(
    page.getByRole("region", { name: "Message reader" }),
  ).toContainText("Body for selection letter 0.");
  expect(await original!.evaluate((n) => n.isConnected)).toBe(true);
  await expect(flag).toBeFocused();
  await page.mouse.up();
  await expect(
    page.getByRole("button", { name: `Unflag ${subject(1)}`, exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: "../artifacts/web/mailbox-held-flag.png" });
});

test("actual paging and body Retry preserve the current reader and reject a late earlier body", async ({
  page,
}) => {
  await page.setViewportSize({ width: 900, height: 640 });
  await seed(page);
  await holdBodies(page);
  await page.getByRole("button", { name: subject(0), exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => (window as any).bodyCalls.length))
    .toBe(1);
  await page.getByRole("button", { name: subject(1), exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => (window as any).bodyCalls.length))
    .toBe(2);
  await page.evaluate(() =>
    (window as any).bodyCalls[1].reject(
      Error("Synthetic cached body unavailable. Retry."),
    ),
  );
  const reader = page.getByRole("region", { name: "Message reader" });
  await expect(reader.getByRole("alert")).toContainText(
    "Synthetic cached body unavailable",
  );
  await page.evaluate(() => (window as any).bodyCalls[0].resolve());
  await expect(reader).not.toContainText("Body for selection letter 0.");
  await page.screenshot({
    path: "../artifacts/web/mailbox-body-retry-compact.png",
  });
  await reader
    .getByRole("button", { name: "Retry message", exact: true })
    .click();
  await expect
    .poll(() => page.evaluate(() => (window as any).bodyCalls.length))
    .toBeGreaterThanOrEqual(3);
  await page.evaluate(() => (window as any).bodyCalls.at(-1).resolve());
  await expect(reader).toContainText("Body for selection letter 1.");
  await page.getByRole("button", { name: "Next page", exact: true }).click();
  await expect(
    page.getByRole("button", { name: subject(50), exact: true }),
  ).toBeVisible();
  await expect(reader).toContainText("Body for selection letter 1.");
  await expect(page.locator(".mail-row")).toHaveCount(50);
});

test("current page failures have an actual Retry and obsolete search errors cannot replace a newer query", async ({
  page,
}) => {
  await seed(page);
  await page.evaluate(async () => {
    const path = "/src/mailbox_worker_client.ts";
    const { MailboxWorkerClient } = await import(path);
    const original = MailboxWorkerClient.prototype.page,
      calls: any[] = [];
    MailboxWorkerClient.prototype.page = async function (q: any) {
      const result = await original.call(this, q);
      return new Promise((resolve, reject) =>
        calls.push({ q, resolve: () => resolve(result), reject }),
      );
    };
    Object.assign(window, { pageCalls: calls });
  });
  await page.getByRole("button", { name: "Next page", exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => (window as any).pageCalls.length))
    .toBe(1);
  await page.evaluate(() =>
    (window as any).pageCalls[0].reject(
      Error("Synthetic page unavailable. Retry."),
    ),
  );
  await expect(page.getByRole("alert")).toContainText(
    "Synthetic page unavailable",
  );
  await page.getByRole("button", { name: "Retry page", exact: true }).click();
  await expect(page.getByText("Loading cached mail…")).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => (window as any).pageCalls.length))
    .toBe(2);
  await page.evaluate(() => (window as any).pageCalls[1].resolve());
  await expect(
    page.getByRole("button", { name: subject(50), exact: true }),
  ).toBeVisible();
  const search = page.getByRole("textbox", { name: "Search conversations" });
  await search.fill("letter 012");
  await expect
    .poll(() => page.evaluate(() => (window as any).pageCalls.length))
    .toBe(3);
  await search.fill("letter 099");
  await expect(search).toHaveValue("letter 099");
  // Debounce acknowledgement is observed at the control's live value below;
  // let the actual 100 ms timer elapse before releasing the previous request.
  await page.waitForTimeout(150);
  await page.evaluate(() =>
    (window as any).pageCalls[2].reject(Error("Obsolete query failure")),
  );
  await expect
    .poll(() => page.evaluate(() => (window as any).pageCalls.length))
    .toBe(4);
  await page.evaluate(() => (window as any).pageCalls[3].resolve());
  await expect(
    page.getByRole("button", { name: subject(99), exact: true }),
  ).toBeVisible();
  await expect(page.locator(".mail-row")).toHaveCount(1);
  await expect(page.getByRole("alert")).toHaveCount(0);
});
