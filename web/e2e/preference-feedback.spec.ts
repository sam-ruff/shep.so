import { expect, test, type Page } from "@playwright/test";
import { seed, profile, subject } from "./mailbox-fixture";

async function navigate(page: Page, name: string) {
  const target = page.getByRole("button", { name, exact: true });
  if (!(await target.isVisible()))
    await page
      .getByRole("button", { name: "Toggle navigation", exact: true })
      .click();
  await target.click();
}

for (const dark of [false, true]) {
  test.describe(
    dark ? "compact dark preferences" : "compact light preferences",
    () => {
      test.use({
        viewport: { width: 390, height: 844 },
        colorScheme: dark ? "dark" : "light",
      });
      test("failed authoritative save survives navigation and unrelated completion, Retry saves current intent", async ({
        page,
      }) => {
        await seed(page);
        await page.evaluate(async () => {
          const path = "/src/provider.ts",
            { GatewayRepository } = await import(path);
          GatewayRepository.prototype.mutateWithReceipt = async function () {
            await new Promise<void>((resolve) => {
              (window as any).releasePreferenceMail = resolve;
            });
            throw Error("Fixture unrelated mail failure");
          };
          const set = Storage.prototype.setItem;
          (window as any).failPreferences = true;
          Storage.prototype.setItem = function (key: string, value: string) {
            if (
              (window as any).failPreferences &&
              key.startsWith("shep.profile-preferences.v1.")
            )
              throw Error("Fixture authoritative settings full");
            return set.call(this, key, value);
          };
        });
        await page
          .getByRole("button", { name: `Flag ${subject(0)}`, exact: true })
          .click();
        await expect
          .poll(() =>
            page.evaluate(() => typeof (window as any).releasePreferenceMail),
          )
          .toBe("function");
        await navigate(page, "Preferences");
        await page
          .getByRole("combobox", { name: "Theme", exact: true })
          .selectOption(dark ? "dark" : "light");
        const feedback = page.getByRole("region", {
          name: "Preference saving",
          exact: true,
        });
        await expect(feedback.getByRole("status")).toContainText(
          "could not be saved",
        );
        await page
          .getByRole("combobox", { name: "Preview lines", exact: true })
          .selectOption("4");
        await navigate(page, "Mail");
        await navigate(page, "Preferences");
        await expect(feedback.getByRole("status")).toContainText(
          "could not be saved",
        );
        await expect(
          page.getByRole("combobox", { name: "Preview lines", exact: true }),
        ).toHaveValue("4");
        const leaving = page.waitForEvent("dialog");
        await page.evaluate(() => {
          setTimeout(() => location.reload(), 0);
        });
        const warning = await leaving;
        expect(warning.type()).toBe("beforeunload");
        await warning.dismiss();
        await expect(feedback.getByRole("status")).toContainText(
          "could not be saved",
        );
        await page.screenshot({
          path: `../artifacts/web/preference-save-error-compact-${dark ? "dark" : "light"}.png`,
        });
        let signouts = 0;
        await page.route("**/api/logout", (route) => {
          signouts++;
          return route.fulfill({ status: 204 });
        });
        await navigate(page, "Sign out");
        await expect(feedback.getByRole("status")).toContainText(
          "could not be saved",
        );
        expect(signouts).toBe(0);
        await page.evaluate(() => {
          (window as any).failPreferences = false;
        });
        const retry = feedback.getByRole("button", {
          name: "Retry preference save",
          exact: true,
        });
        await retry.scrollIntoViewIfNeeded();
        const box = await retry.boundingBox();
        expect(box).not.toBeNull();
        await page.mouse.move(
          box!.x + box!.width / 2,
          box!.y + box!.height / 2,
        );
        await page.mouse.down();
        await page.evaluate(() => (window as any).releasePreferenceMail());
        await expect(page.locator(".error-banner")).toContainText(
          "Could not confirm the update",
        );
        await expect(feedback.getByRole("status")).toContainText(
          "could not be saved",
        );
        await page.screenshot({
          path: `../artifacts/web/preference-independent-error-compact-${dark ? "dark" : "light"}.png`,
        });
        await page.mouse.up();
        await expect(feedback.getByRole("status")).toHaveText(
          "Preferences saved on this browser",
        );
        await expect(page.locator(".error-banner")).toContainText(
          "Could not confirm the update",
        );
        await page
          .getByRole("button", { name: "Dismiss error", exact: true })
          .click();
        await expect(page.locator(".error-banner")).toHaveCount(0);
        await expect(feedback.getByRole("status")).toHaveText(
          "Preferences saved on this browser",
        );
        const saved = await page.evaluate(
          (profile) =>
            JSON.parse(
              localStorage.getItem(`shep.profile-preferences.v1.${profile}`)!,
            ),
          profile,
        );
        expect(saved.preferences).toMatchObject({
          appearance: dark ? "dark" : "light",
          previewLines: 4,
        });
        expect(saved.revisions).toMatchObject({
          appearance: 1,
          preview_lines: 1,
        });
        await page.reload();
        await navigate(page, "Preferences");
        await expect(
          page.getByRole("combobox", { name: "Theme", exact: true }),
        ).toHaveValue(dark ? "dark" : "light");
        await expect(
          page.getByRole("combobox", { name: "Preview lines", exact: true }),
        ).toHaveValue("4");
        await expect(
          feedback.getByRole("button", {
            name: "Retry preference save",
            exact: true,
          }),
        ).toHaveCount(0);
      });
    },
  );
}
