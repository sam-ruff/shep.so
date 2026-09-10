import { expect, test, type Page } from "@playwright/test";
import { seed, subject } from "./mailbox-fixture";

async function pendingFlag(page: Page) {
  await page.route("**/api/capabilities", (route) =>
    route.fulfill({ json: { mail: false, endpoints: [] } }),
  );
  await seed(page);
  await page.evaluate(async () => {
    const path = "/src/provider.ts";
    const { GatewayRepository } = await import(path);
    const mutate = GatewayRepository.prototype.mutateWithReceipt;
    let first = true;
    GatewayRepository.prototype.mutateWithReceipt = async function (
      ...args: any[]
    ) {
      if (!first) return mutate.apply(this, args);
      first = false;
      await new Promise<void>((resolve) =>
        Object.assign(window, { releasePreferenceFlag: resolve }),
      );
      try {
        return await mutate.apply(this, args);
      } finally {
        Object.assign(window, { preferenceFlagFinished: true });
      }
    };
  });
  await page
    .getByRole("button", { name: `Flag ${subject(0)}`, exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => typeof (window as any).releasePreferenceFlag),
    )
    .toBe("function");
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
}

async function finishFlag(page: Page) {
  await page.evaluate(() => (window as any).releasePreferenceFlag());
  await expect
    .poll(() => page.evaluate(() => (window as any).preferenceFlagFinished))
    .toBe(true);
}

test("shortcut capture and modifier input survive a completed mail write and persist after reload", async ({
  page,
}) => {
  await pendingFlag(page);
  const capture = page.getByRole("button", { name: "Remap find", exact: true });
  await capture.click();
  await expect(capture).toHaveText("Press a key…");
  await page.keyboard.down("Control");
  await finishFlag(page);
  await expect(capture).toBeFocused();
  await expect(capture).toHaveText("Press a key…");
  await page.keyboard.press("g");
  await page.keyboard.up("Control");
  await expect(capture).toHaveText("Control+g");
  await page.reload();
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await expect(capture).toHaveText("Control+g");
  await capture.scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "../artifacts/web/preferences-capture-persisted.png",
  });
});

test("a held remap press survives background completion; conflict, cancel and clearing remain usable", async ({
  page,
}) => {
  await page.setViewportSize({ width: 900, height: 640 });
  await pendingFlag(page);
  await page.getByLabel("Theme", { exact: true }).selectOption("dark");
  const capture = page.getByRole("button", { name: "Remap find", exact: true });
  await capture.scrollIntoViewIfNeeded();
  const box = (await capture.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await finishFlag(page);
  await page.mouse.up();
  await expect(capture).toHaveText("Press a key…");
  await page.keyboard.press("Control+p");
  await expect(capture).toHaveText("Already assigned");
  await page.keyboard.press("Escape");
  await expect(capture).toHaveText("Control+f");
  await capture.click();
  await page.keyboard.press("Alt+g");
  await expect(capture).toHaveText("Alt+g");
  await capture.click();
  await expect(capture).toHaveText("Press a key…");
  await page.getByRole("button", { name: "Clear find", exact: true }).click();
  await expect(capture).toHaveText("Disabled");
  await page.reload();
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await expect(capture).toHaveText("Disabled");
  await expect(page.getByLabel("Theme", { exact: true })).toHaveValue("dark");
  await capture.scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "../artifacts/web/preferences-capture-dark.png",
  });
});

test("approve and decline review keys are listed, remappable, conflict-checked and honoured by the group review", async ({
  page,
}) => {
  await page.route("**/api/capabilities", (route) =>
    route.fulfill({ json: { mail: false, endpoints: [] } }),
  );
  await seed(page);
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  const approve = page.getByRole("button", {
    name: "Remap approve review",
    exact: true,
  });
  const decline = page.getByRole("button", {
    name: "Remap decline review",
    exact: true,
  });
  await expect(approve).toHaveText("y");
  await expect(decline).toHaveText("n");
  await approve.click();
  await page.keyboard.press("Control+y");
  await expect(approve).toHaveText("Control+y");
  await decline.click();
  await page.keyboard.press("Control+y");
  await expect(decline).toHaveText("Already assigned");
  await page.keyboard.press("Escape");
  await expect(decline).toHaveText("n");
  await decline.click();
  await page.keyboard.press("Alt+n");
  await expect(decline).toHaveText("Alt+n");
  await decline.scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "../artifacts/web/preferences-review-keys.png",
  });
  await page.getByRole("button", { name: "Mail", exact: true }).click();
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
  const apply = review.getByRole("button", {
    name: "Archive 125 messages",
    exact: true,
  });
  await expect(apply).toBeFocused();
  // The default letters no longer act once remapped.
  await page.keyboard.press("y");
  await page.keyboard.press("n");
  await expect(review).toBeVisible();
  await page.keyboard.press("Alt+n");
  await expect(review).toBeHidden();
  await page
    .getByRole("button", { name: "Select all messages", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Archive selected messages", exact: true })
    .click();
  await expect(apply).toBeFocused();
  await page.keyboard.press("Control+y");
  await expect(review).toBeHidden();
  await expect(page.locator("main > header")).toContainText(
    "0 messages · 0 unread",
  );
  await page.reload();
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await expect(approve).toHaveText("Control+y");
  await expect(decline).toHaveText("Alt+n");
});

test("leaving a capture cancels it without changing another shortcut or preference", async ({
  page,
}) => {
  await pendingFlag(page);
  const find = page.getByRole("button", { name: "Remap find", exact: true });
  const print = page.getByRole("button", { name: "Remap print", exact: true });
  await find.click();
  await finishFlag(page);
  await print.click();
  await expect(find).toHaveText("Control+f");
  await expect(print).toHaveText("Press a key…");
  await page.keyboard.press("Alt+p");
  await expect(print).toHaveText("Alt+p");
  await find.click();
  await page.getByLabel("Theme", { exact: true }).click();
  await page.getByLabel("Theme", { exact: true }).selectOption("dark");
  await expect(find).toHaveText("Control+f");
  await find.click();
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await expect(find).toHaveText("Control+f");
  await page.reload();
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await expect(find).toHaveText("Control+f");
  await expect(print).toHaveText("Alt+p");
  await expect(page.getByLabel("Theme", { exact: true })).toHaveValue("dark");
});
