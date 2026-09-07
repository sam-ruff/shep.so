import { test, expect } from "@playwright/test";

const subjects = [
  "A little room for good ideas",
  "Your week, a little clearer",
];
test("quick archives share counted Undo and restored feedback expires", async ({
  page,
}) => {
  await page.goto("/preview.html");
  for (const subject of subjects) {
    await page.getByRole("button", { name: subject, exact: true }).click();
    await page
      .getByRole("region", { name: "Message reader" })
      .getByRole("button", { name: "Archive", exact: true })
      .click();
  }
  const toast = page.getByRole("status", { name: "Move notification" });
  await expect(toast).toContainText("Archived 2 messages");
  await page.screenshot({ path: "../artifacts/web/counted-archive.png" });
  await toast.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(toast).toContainText("Restored 2 messages");
  for (const subject of subjects)
    await expect(
      page.getByRole("button", { name: subject, exact: true }),
    ).toBeVisible();
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.getByRole("status", { name: "Mail status" })).toContainText(
    "Preview refreshed",
  );
  await expect(toast).toContainText("Restored 2 messages");
  await expect(toast).toHaveCount(0, { timeout: 7000 });
  await expect(
    page.getByRole("button", { name: "Undo", exact: true }),
  ).toHaveCount(0);
});

test("reader dismissal stays dismissed through acknowledgment and refresh", async ({
  page,
}) => {
  await page.goto("/preview.html");
  await page.getByRole("button", { name: subjects[0], exact: true }).dblclick();
  await page
    .getByRole("region", { name: "Message reader" })
    .getByRole("button", { name: "Archive", exact: true })
    .click();
  const toast = page.getByRole("status", { name: "Move notification" });
  await expect(toast).toContainText("Archived 1 message");
  await toast
    .getByRole("button", { name: "Dismiss move notification" })
    .click();
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.getByRole("status", { name: "Mail status" })).toContainText(
    "Preview refreshed",
  );
  await expect(toast).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page
    .getByRole("button", { name: "Archive", exact: true })
    .first()
    .click();
  await expect(
    page.getByRole("button", { name: subjects[0], exact: true }),
  ).toBeVisible();
});
