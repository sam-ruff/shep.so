import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Captures reviewed side by side with the desktop harness screenshots. Each
// scheme records the mailbox with an open reader, the composer, Preferences,
// the empty search state and the error notice at the desktop window size.
const subject = "A little room for good ideas";
const out = "../artifacts/web/visual-parity";

async function checkAccessibility(page: Page) {
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
}

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 920 });
  await page.goto("/preview.html");
});

for (const theme of ["light", "dark"] as const) {
  test(`visual parity captures ${theme}`, async ({ page }) => {
    await page
      .getByRole("button", { name: "Preferences", exact: true })
      .click();
    await page
      .getByRole("combobox", { name: "Theme", exact: true })
      .selectOption(theme);
    await checkAccessibility(page);
    await page.screenshot({ path: `${out}/preferences-${theme}.png` });

    await page.getByRole("button", { name: "Mail", exact: true }).click();
    await page.getByRole("button", { name: subject, exact: true }).click();
    await expect(page.getByRole("heading", { name: subject })).toBeVisible();
    await checkAccessibility(page);
    await page.screenshot({ path: `${out}/inbox-reader-${theme}.png` });

    await page
      .getByRole("button", { name: "New message", exact: true })
      .click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await checkAccessibility(page);
    await page.screenshot({ path: `${out}/composer-${theme}.png` });
    await page.getByRole("button", { name: "Save draft", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);

    await page
      .getByRole("textbox", { name: "Search conversations" })
      .fill("zzzz");
    await expect(page.getByText("No matching mail")).toBeVisible();
    await checkAccessibility(page);
    await page.screenshot({ path: `${out}/empty-${theme}.png` });
    await page.getByRole("textbox", { name: "Search conversations" }).fill("");
  });

  test(`error notice ${theme}`, async ({ page }) => {
    await page.goto("/preview.html?fail");
    await page
      .getByRole("button", { name: "Preferences", exact: true })
      .click();
    await page
      .getByRole("combobox", { name: "Theme", exact: true })
      .selectOption(theme);
    await page.getByRole("button", { name: "Mail", exact: true }).click();
    await page.getByRole("button", { name: "Refresh", exact: true }).click();
    await expect(page.getByRole("alert")).toContainText("Fixture rejection");
    await checkAccessibility(page);
    await page.screenshot({ path: `${out}/error-${theme}.png` });
  });
}

test("drawer sidebar keeps the desktop look on a phone-sized window", async ({
  page,
}) => {
  await page.setViewportSize({ width: 412, height: 892 });
  await page
    .getByRole("button", { name: "Toggle navigation", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "New message", exact: true }),
  ).toBeVisible();
  await checkAccessibility(page);
  await page.screenshot({ path: `${out}/drawer-compact-light.png` });
});
