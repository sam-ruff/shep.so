import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
const subject = "A little room for good ideas";
test.beforeEach(async ({ page }) => {
  await page.goto("/preview.html");
});
test("reader, select text, full window, Escape and reply refusal", async ({
  page,
}) => {
  await page.getByRole("button", { name: subject, exact: true }).dblclick();
  await expect(page.getByRole("heading", { name: subject })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Close full reader" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Reply", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "To", exact: true }),
  ).toHaveValue("alex@example.test");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.getByRole("dialog")).toContainText("Preview cannot send mail");
  await expect(
    page.getByRole("textbox", { name: "Message", exact: true }),
  ).toHaveValue(/The first sketches/);
});
test("immediate archive, navigate pending, undo", async ({ page }) => {
  await page.getByRole("button", { name: subject, exact: true }).click();
  await page
    .getByRole("button", { name: "Archive", exact: true })
    .last()
    .click();
  await expect(
    page.getByRole("button", { name: subject, exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(
    page.getByRole("button", { name: subject, exact: true }),
  ).toBeVisible();
});
test("failed archive restores and remains actionable", async ({ page }) => {
  await page.goto("/preview.html?fail=1");
  await page.getByRole("button", { name: subject, exact: true }).click();
  await page
    .getByRole("button", { name: "Archive", exact: true })
    .last()
    .click();
  await expect(page.getByRole("alert")).toContainText("restored");
  await expect(
    page.getByRole("button", { name: subject, exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Calendar", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "September 2026" }),
  ).toBeVisible();
});
test("search typing does not run mail shortcuts", async ({ page }) => {
  await page.getByRole("button", { name: subject, exact: true }).click();
  await page
    .getByRole("textbox", { name: "Search conversations" })
    .fill("Morgan");
  await expect(
    page.getByRole("button", { name: subject, exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page
    .getByRole("textbox", { name: "Search conversations" })
    .fill("zzzz");
  await expect(page.getByText("No matching mail")).toBeVisible();
});
test("flag, filter and sort use native controls", async ({ page }) => {
  await page
    .getByRole("button", { name: `Flag ${subject}`, exact: true })
    .click();
  await page
    .getByRole("combobox", { name: "Filter", exact: true })
    .selectOption("Flagged");
  await expect(
    page.getByRole("button", { name: subject, exact: true }),
  ).toBeVisible();
  await page
    .getByRole("combobox", { name: "Sort", exact: true })
    .selectOption("Oldest first");
  await expect(page.locator(".mail-row").first()).toContainText(
    "Coffee on Thursday?",
  );
});
test("theme, preview and shortcuts survive reload", async ({ page }) => {
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page
    .getByRole("combobox", { name: "Theme", exact: true })
    .selectOption("dark");
  await page
    .getByRole("combobox", { name: "Preview lines", exact: true })
    .selectOption("0");
  await page.getByRole("button", { name: "Clear move", exact: true }).click();
  await page.reload();
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await expect(
    page.getByRole("combobox", { name: "Theme", exact: true }),
  ).toHaveValue("dark");
  await expect(
    page.getByRole("combobox", { name: "Preview lines", exact: true }),
  ).toHaveValue("0");
  await expect(
    page.getByRole("button", { name: "Remap move", exact: true }),
  ).toHaveText("Disabled");
});
test("dragging divider persists and keyboard resizing works", async ({
  page,
}) => {
  const divider = page.getByRole("separator", { name: "Sidebar width" });
  const box = (await divider.boundingBox())!;
  await page.mouse.move(box.x + 2, box.y + 100);
  await page.mouse.down();
  await page.mouse.move(box.x + 52, box.y + 100);
  await page.mouse.up();
  await expect(divider).toHaveAttribute("aria-valuenow", "268");
  await page.reload();
  await expect(divider).toHaveAttribute("aria-valuenow", "268");
  await divider.focus();
  await page.keyboard.press("ArrowLeft");
  await expect(divider).toHaveAttribute("aria-valuenow", "258");
});
test("calendar writable and read-only controls", async ({ page }) => {
  await page.getByRole("button", { name: "Calendar", exact: true }).click();
  await page
    .getByRole("button", { name: "Coffee with Jamie", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Event title" })
    .fill("Coffee at eleven");
  await page.getByRole("button", { name: "Save event" }).click();
  await expect(
    page.getByRole("button", { name: "Coffee at eleven" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Team day", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Event title" }),
  ).toHaveAttribute("readonly", "");
  await expect(page.getByRole("button", { name: "Save event" })).toHaveCount(0);
});
test("production entry requires beta login and contains no fixture mail", async ({
  page,
}) => {
  await page.route("**/api/session", (route) =>
    route.fulfill({
      status: 401,
      contentType: "application/json",
      body: '{"error":"Sign in"}',
    }),
  );
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Shep private beta" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: subject, exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("link", { name: "Continue with Google" }),
  ).toHaveAttribute("href", "/auth/start");
});
test("verified session opens empty client and exposes sign out", async ({
  page,
}) => {
  await page.route("**/api/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        email: "owner@example.test",
        csrf: "a".repeat(43),
        user_id: "u".repeat(43),
      }),
    }),
  );
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Welcome to Shep" }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Sign out" })).toBeVisible();
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Add a mail account in Preferences");
});
for (const theme of ["light", "dark"])
  for (const [width, height] of [
    [1440, 920],
    [900, 640],
  ])
    test(`layout ${theme} ${width}x${height}`, async ({ page }) => {
      await page.setViewportSize({ width, height });
      await page
        .getByRole("button", { name: "Preferences", exact: true })
        .click();
      await page
        .getByRole("combobox", { name: "Theme", exact: true })
        .selectOption(theme);
      await page.getByRole("button", { name: "Mail", exact: true }).click();
      await page.getByRole("button", { name: subject, exact: true }).click();
      await expect
        .poll(() =>
          page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
        )
        .toBe(true);
      expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
      await page.screenshot({ path: `../artifacts/web/${theme}-${width}.png` });
    });
