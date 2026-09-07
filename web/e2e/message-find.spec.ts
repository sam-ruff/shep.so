import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
// @ts-expect-error Shared runnable scenario is also imported by the Rust runner.
import { messageFindFlow } from "./message-find-flow.mjs";
const subject = "A little room for good ideas";
test("Find visible text, wrapping, Unicode, case, quote scope, navigation and compact themes", async ({
  page,
}) => {
  await page.goto("/preview.html?find=1");
  await page.getByRole("button", { name: subject, exact: true }).click();
  await messageFindFlow(page);
  for (const [theme, width, height] of [
    ["light", 1440, 920],
    ["dark", 900, 640],
  ] as const) {
    await page
      .getByRole("button", { name: "Preferences", exact: true })
      .click();
    await page.getByLabel("Theme", { exact: true }).selectOption(theme);
    await page.getByRole("button", { name: "Mail", exact: true }).click();
    await page.setViewportSize({ width, height });
    await page
      .getByRole("button", { name: "Find in message", exact: true })
      .click();
    await expect(page.locator(".find-status")).toHaveText("1 of 3");
    await page.getByRole("button", { name: "Previous match" }).click();
    await expect(page.locator(".find-status")).toHaveText("3 of 3");
    expect(
      (
        await new AxeBuilder({ page })
          .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
          .analyze()
      ).violations,
    ).toEqual([]);
    await page.screenshot({
      path: `../artifacts/web/find-${theme}-${width}.png`,
    });
    await page.getByRole("button", { name: "Close Find" }).click();
  }
});
test("Find remapping, disabling, input isolation, message changes and full-reader Escape", async ({
  page,
}) => {
  await page.goto("/preview.html?find=1");
  await page.getByRole("button", { name: subject, exact: true }).dblclick();
  await page.keyboard.press("Control+f");
  const input = page.getByRole("textbox", { name: "Find in message" });
  await expect(input).toBeFocused();
  await input.fill("alpha");
  await expect(page.locator(".find-status")).toHaveText("1 of 2");
  await input.fill("m");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await input.press("Escape");
  await expect(input).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Close full reader" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("button", { name: "Close full reader" }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page.getByRole("button", { name: "Remap find", exact: true }).click();
  await page.keyboard.press("Control+g");
  await page.reload();
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Remap find", exact: true }),
  ).toHaveText("Control+g");
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await page.getByRole("button", { name: subject, exact: true }).click();
  await page.keyboard.press("Control+g");
  await expect(input).toBeFocused();
  await input.fill("alpha");
  await expect(page.locator(".find-status")).toHaveText("1 of 2");
  await page
    .locator(".mail-row")
    .filter({ hasNotText: subject })
    .first()
    .getByRole("button")
    .first()
    .click();
  await expect(page.locator(".find-status")).toHaveText("No matches");
  await expect(page.locator(".find-hit")).toHaveCount(0);
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page.getByRole("button", { name: "Clear find", exact: true }).click();
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await page.getByRole("button", { name: "Close Find" }).click();
  await page.keyboard.press("Control+g");
  await expect(input).toHaveCount(0);
});

test("Find worker startup failure stays visible and Retry recovers the current query", async ({
  page,
}) => {
  const worker = /\/src\/search_worker\.ts\?/;
  await page.route(worker, (route) => route.abort("failed"));
  await page.goto("/preview.html?find=1");
  await page.getByRole("button", { name: subject, exact: true }).click();
  await page
    .getByRole("button", { name: "Find in message", exact: true })
    .click();
  const input = page.getByRole("textbox", { name: "Find in message" });
  await input.fill("alpha");
  await expect(page.locator(".find-status")).toContainText("Retry Find");
  await expect(page.getByRole("button", { name: "Next match" })).toBeDisabled();
  await page.unroute(worker);
  await page.getByRole("button", { name: "Retry Find", exact: true }).click();
  await expect(page.locator(".find-status")).toHaveText("1 of 2");
  await expect(input).toHaveValue("alpha");
});
