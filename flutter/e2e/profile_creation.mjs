import assert from "node:assert/strict";
import { withProfileHarness } from "./profile_harness.mjs";
await withProfileHarness(
  process.argv[2] ?? "web",
  "creation",
  async ({
    mode,
    page,
    driver,
    steps,
    labels,
    reveal,
    tap,
    wait,
    capture,
    theme,
  }) => {
    await wait("A little room for good ideas");
    await tap("Preferences");
    await wait("Appearance");
    await theme("Light");
    await reveal("Saved Google profiles", { button: true });
    await tap("Saved Google profiles");
    await reveal("Find profiles", { button: true });
    await tap("Find profiles");
    await wait("Profile discovery complete");
    await reveal("Create profile", { button: true, direction: -1 });
    await tap("Create profile");
    await wait("Profile name");
    if (mode === "web")
      await page
        .getByRole("textbox", { name: /Profile name/ })
        .fill("Shared controls");
    else {
      const input = await driver.$(
        'android=new UiSelector().className("android.widget.EditText")',
      );
      await input.click();
      await input.setValue("Shared controls");
      await wait("Shared controls");
      if (await driver.isKeyboardShown()) await driver.hideKeyboard();
    }
    await reveal("Review profile", { button: true });
    await tap("Review profile");
    await wait("Profile publication");
    await wait("Shared controls");
    await reveal("Appearance");
    await capture("creation-review-light");
    await reveal("Saved account 1", { button: true });
    await tap("Saved account 1");
    await wait("Incoming: IMAP");
    assert.ok((await labels()).includes("Passwords are not included"));
    await capture("creation-account-detail-light");
    await tap("Close");
    steps.push("creation-review-account-details");
    await reveal("Publish profile", { button: true });
    await tap("Publish profile");
    await wait("Profile upload could not be confirmed");
    await reveal("Resume publication", { button: true });
    await capture("creation-error-light");
    await tap("Resume publication");
    await wait("Profile saved to Google");
    await reveal("Profile saved to Google");
    await capture("creation-complete-light");
    assert.ok((await labels()).includes("5 of 5 profile files confirmed"));
    steps.push("creation-upload-retry");
    await tap("Back");
    await wait("Profiles and sync");
    await tap("Back");
    await wait("Preferences");
    await theme("Dark");
    await reveal("Saved Google profiles", { button: true });
    await tap("Saved Google profiles");
    await reveal("Shared controls", { button: true });
    await tap("Shared controls");
    await wait("Profile saved to Google");
    await reveal("Profile saved to Google");
    await capture("creation-complete-dark");
    steps.push("creation-dark-appearance");
    await tap("Back");
    await wait("Profiles and sync");
    await tap("Back");
    await tap("Mail");
    await wait("A little room for good ideas");
    steps.push("creation-mail-navigation");
  },
);
