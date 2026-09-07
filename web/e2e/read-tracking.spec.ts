import { test, expect } from "@playwright/test";

const first = "A little room for good ideas";
const second = "Your week, a little clearer";
test("deliberate reading finishes on navigation and explicit unread survives leaving", async ({
  page,
}) => {
  await page.goto("/preview.html");
  const row = (subject: string) =>
    page
      .locator(".mail-row")
      .filter({
        has: page.getByRole("button", { name: subject, exact: true }),
      });
  await expect(row(first)).toHaveClass(/unread/);
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Preview refreshed");
  await expect(row(first)).toHaveClass(/unread/);
  await page.getByRole("button", { name: first, exact: true }).click();
  await expect(row(first)).toHaveClass(/unread/);
  await expect(
    page.getByRole("button", { name: "Mark read", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: second, exact: true }).click();
  await expect(row(first)).not.toHaveClass(/unread/);
  await expect(row(second)).toHaveClass(/unread/);
  await page.getByRole("button", { name: "Mark read", exact: true }).click();
  await page.getByRole("button", { name: "Mark unread", exact: true }).click();
  await page.getByRole("button", { name: first, exact: true }).click();
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Preview refreshed");
  await expect(row(second)).toHaveClass(/unread/);
  await page.screenshot({ path: "../artifacts/web/read-on-leave.png" });
});

test("failed read while navigating restores the old message and keeps the new reader usable", async ({
  page,
}) => {
  await page.goto("/preview.html?fail=1");
  await page.getByRole("button", { name: first, exact: true }).click();
  await page.getByRole("button", { name: second, exact: true }).click();
  await expect(page.getByRole("heading", { name: second })).toBeVisible();
  await expect(page.getByRole("alert")).toContainText("restored");
  await expect(
    page
      .locator(".mail-row")
      .filter({ has: page.getByRole("button", { name: first, exact: true }) }),
  ).toHaveClass(/unread/);
  await expect(
    page.getByRole("button", { name: "Mark read", exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: "../artifacts/web/read-on-leave-failure.png" });
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await expect(
    page.getByRole("combobox", { name: "Theme", exact: true }),
  ).toBeVisible();
});
