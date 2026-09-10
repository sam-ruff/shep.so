import { expect } from "@playwright/test";
import assert from "node:assert/strict";
// Used by preview and authenticated Rust HTTPS scenarios, including offline.
export async function messageFindFlow(page) {
  const reader = page.getByRole("region", { name: "Message reader" });
  const input = reader.getByRole("textbox", { name: "Find in message" });
  const status = reader.locator(".find-status");
  const query = async (value, result) => {
    await input.fill(value);
    await expect(status).toHaveText(result);
  };
  const visible = async () => {
    await expect
      .poll(async () =>
        page.locator(".find-hit.active").evaluate((mark) => {
          const hit = mark.getBoundingClientRect(),
            viewport = mark.closest(".reader-content").getBoundingClientRect();
          return hit.top >= viewport.top && hit.bottom <= viewport.bottom;
        }),
      )
      .toBe(true);
  };
  await reader
    .getByRole("button", { name: "Find in message", exact: true })
    .click();
  await query("alpha", "1 of 2");
  await visible();
  await input.press("Enter");
  await expect(status).toHaveText("2 of 2");
  await visible();
  await expect(input).toBeFocused();
  await input.press("Enter");
  await expect(status).toHaveText("1 of 2");
  await input.press("Shift+Enter");
  await expect(status).toHaveText("2 of 2");
  await reader.getByRole("button", { name: "Match case" }).click();
  await expect(status).toHaveText("No matches");
  await expect(
    reader.getByRole("button", { name: "Next match" }),
  ).toBeDisabled();
  await reader.getByRole("button", { name: "Match case" }).click();
  await expect(status).toHaveText("1 of 2");
  await query("Alpha wraps over lines.", "1 of 1");
  await visible();
  assert.match(
    await reader.locator(".find-hit.active").innerText(),
    /Alpha wraps\s+over lines\./,
  );
  await query("CAFÉ", "1 of 3");
  await query("alpha", "1 of 2");
  await reader.getByText("Quoted history", { exact: true }).click();
  await expect(status).toHaveText("1 of 3");
  await reader.getByRole("button", { name: "Previous match" }).click();
  await expect(status).toHaveText("3 of 3");
  await visible();
  await reader.getByRole("button", { name: "Close Find" }).click();
  await expect(input).toHaveCount(0);
  await expect(reader.locator(".find-hit")).toHaveCount(0);
}
