import { expect, test, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import {
  DriveFixture,
  publishedProfile,
  seedAccounts,
  uuid,
} from "./profiles-fixture";

const out = "../artifacts/web/profiles";
async function openPreferences(page: Page) {
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "Google connection" }),
  ).toBeVisible();
}
async function axe(page: Page) {
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
}
/// Scroll the reviewed region into view before a viewport capture.
async function capture(page: Page, name: string, region: string) {
  await page.getByRole("region", { name }).scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${out}/${region}.png` });
}
async function connected(fixture: DriveFixture) {
  fixture.connected = true;
  fixture.granted = {
    drive: true,
    calendar_read: false,
    calendar_write: false,
  };
}
const discoveryStatus = (page: Page) =>
  page.getByRole("status", { name: "Discovery status" });

test("consent choices: requested versus saved permissions, denial keeps the grant, disconnect cleans up locally", async ({
  page,
}) => {
  const fixture = new DriveFixture();
  await fixture.install(page);
  await page.goto("/");
  await openPreferences(page);
  const card = page.getByRole("region", { name: "Google connection" });
  await expect(card).toContainText("Not connected");
  await expect(card).toContainText(
    "Live Google provider access is not connected on this beta server",
  );
  await expect(card).toContainText(
    "Google beta sign-in identifies you; it does not grant Drive or Calendar access.",
  );
  await page
    .getByRole("combobox", { name: "Calendar access", exact: true })
    .selectOption("read");
  await expect(card).toContainText(
    "Requested at next sign-in: Drive app data and Calendar (read)",
  );
  await capture(page, "Google connection", "consent-light");
  await axe(page);
  // Denied consent: the browser comes back with the saved connection unchanged.
  fixture.consentOutcome = "denied";
  await page
    .getByRole("button", { name: "Connect Google", exact: true })
    .click();
  await expect(
    page.getByRole("region", { name: "Google connection" }),
  ).toContainText(
    "Google did not grant the requested access. The saved connection is unchanged.",
  );
  expect(fixture.connects).toEqual([{ drive: true, calendar: "read" }]);
  await expect(
    page.getByRole("region", { name: "Google connection" }),
  ).toContainText("Not connected");
  // A changed choice does not change saved access until sign-in succeeds.
  await page
    .getByRole("checkbox", { name: "Drive app data for profile sync" })
    .uncheck();
  await page
    .getByRole("combobox", { name: "Calendar access", exact: true })
    .selectOption("edit");
  fixture.consentOutcome = "connected";
  await page
    .getByRole("button", { name: "Connect Google", exact: true })
    .click();
  await expect(
    page.getByRole("region", { name: "Google connection" }),
  ).toContainText("Google connected.");
  await expect(
    page.getByRole("region", { name: "Google connection" }),
  ).toContainText("Saved permissions: Calendar (edit)");
  expect(fixture.connects.at(-1)).toEqual({ drive: false, calendar: "edit" });
  await expect(
    page.getByRole("region", { name: "Synced profiles" }),
  ).toContainText("Connect Google with Drive app data to find profiles");
  // Reconnect with Drive enables discovery automatically.
  await page
    .getByRole("checkbox", { name: "Drive app data for profile sync" })
    .check();
  await page
    .getByRole("button", { name: "Reconnect Google", exact: true })
    .click();
  await expect(
    page.getByRole("region", { name: "Google connection" }),
  ).toContainText(
    "Saved permissions: Drive app data and Calendar (edit) · Drive identity verified",
  );
  await expect(discoveryStatus(page)).toContainText(
    "Discovery complete: 0 files verified.",
  );
  await page
    .getByRole("combobox", { name: "Theme", exact: true })
    .selectOption("dark");
  await capture(page, "Google connection", "consent-connected-dark");
  await axe(page);
  await page
    .getByRole("button", { name: "Disconnect Google", exact: true })
    .click();
  await expect(
    page.getByRole("region", { name: "Google connection" }),
  ).toContainText("Google disconnected from this browser");
  await expect(
    page.getByRole("region", { name: "Google connection" }),
  ).toContainText("Not connected");
  expect(fixture.disconnects).toBe(1);
  await expect(
    page.getByRole("button", { name: "Retry local cleanup" }),
  ).toHaveCount(0);
});

test("discovery verifies pages of profiles, saves an incomplete listing as a failure and retries from the same step", async ({
  page,
}) => {
  const fixture = new DriveFixture();
  await connected(fixture);
  for (let i = 0; i < 52; i++)
    fixture.add(
      publishedProfile(uuid(100 + i), 1000 + i * 3, `Profile ${i}`)[0],
    );
  for (const op of publishedProfile(uuid(1), 10, "Laptop")) fixture.add(op);
  fixture.incompleteOnce = true;
  await fixture.install(page);
  await page.goto("/");
  await openPreferences(page);
  await expect(discoveryStatus(page)).toContainText(
    "Discovery stopped: Drive returned an incomplete listing.",
  );
  await expect(
    page.getByRole("region", { name: "Synced profiles" }),
  ).toContainText("No verified profiles yet.");
  await expect(
    page.getByRole("button", { name: "Create profile", exact: true }),
  ).toBeDisabled();
  await capture(page, "Synced profiles", "discovery-failed-light");
  await page
    .getByRole("button", { name: "Retry discovery", exact: true })
    .click();
  await expect(discoveryStatus(page)).toContainText(
    "Discovery complete: 55 files verified.",
  );
  const list = page
    .getByRole("region", { name: "Synced profiles" })
    .getByRole("listitem");
  await expect(list).toHaveCount(50);
  await expect(list.first()).toContainText("Laptop");
  await expect(list.first()).toContainText(
    "1 account · 3 settings · 3 files · Ready",
  );
  await expect(list.nth(1)).toContainText("Setup incomplete");
  await page.getByRole("button", { name: "Next page", exact: true }).click();
  await expect(list).toHaveCount(3);
  await page.getByRole("button", { name: "First page", exact: true }).click();
  await expect(list).toHaveCount(50);
  await axe(page);
  // Reload restores the saved catalog without touching Google.
  const calls = fixture.calls.length;
  await page.reload();
  await openPreferences(page);
  await expect(discoveryStatus(page)).toContainText(
    "Discovery complete: 55 files verified.",
  );
  await expect(list).toHaveCount(50);
  expect(fixture.calls.length).toBe(calls);
  await page
    .getByRole("combobox", { name: "Theme", exact: true })
    .selectOption("dark");
  await capture(page, "Synced profiles", "discovery-dark");
  await axe(page);
});

test("publication review, lost upload reply, retry, pause with mail browsing and resume", async ({
  page,
}) => {
  const fixture = new DriveFixture();
  await connected(fixture);
  await fixture.install(page);
  await seedAccounts(page, ["work@example.test", "home@example.test"]);
  await page.goto("/");
  await openPreferences(page);
  await expect(discoveryStatus(page)).toContainText("Discovery complete");
  await page
    .getByRole("button", { name: "Create profile", exact: true })
    .click();
  const dialog = page.getByRole("dialog", { name: "Create profile" });
  await page
    .getByRole("textbox", { name: "Profile name" })
    .fill("Browser profile");
  await page.getByRole("button", { name: "Review", exact: true }).click();
  await expect(dialog).toContainText("Profile “Browser profile”");
  await page
    .getByRole("button", { name: "Details for work@example.test" })
    .click();
  await expect(dialog).toContainText("IMAP mail.example.test:993");
  await page
    .getByRole("checkbox", { name: "Publish home@example.test" })
    .uncheck();
  await page.getByRole("checkbox", { name: "Preview lines: 2" }).uncheck();
  await page.screenshot({ path: `${out}/publication-review-light.png` });
  await axe(page);
  fixture.loseCreateReply = true;
  await page.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(dialog.getByRole("alert")).toContainText(
    "Google Drive did not answer",
  );
  await expect(
    dialog.getByRole("status", { name: "Publication progress" }),
  ).toContainText("3/3 operations staged, 0 uploaded.");
  const creates = fixture.calls.filter((c) => c === "create").length;
  // Hold the next create so Pause can finish that accepted step.
  let release!: () => void;
  fixture.holdCreate = new Promise<void>((resolve) => (release = resolve));
  await page
    .getByRole("button", { name: "Retry publication", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Pause publication", exact: true })
    .click();
  await page.getByRole("button", { name: "Browse mail", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await page.getByRole("button", { name: "Mail", exact: true }).first().click();
  await expect(page.getByRole("heading", { name: "Inbox" })).toBeVisible();
  release();
  await openPreferences(page);
  await expect(
    page.getByRole("status", { name: "Publication status" }),
  ).toContainText("Paused “Browser profile”: 3/3 staged, 2 uploaded.");
  fixture.holdCreate = null;
  await page
    .getByRole("button", { name: "Open publication", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Resume publication", exact: true })
    .click();
  await expect(
    dialog.getByRole("status", { name: "Publication progress" }),
  ).toContainText("Published 3 files to Google app data.");
  // The lost reply never produced a duplicate file.
  expect(fixture.calls.filter((c) => c === "create").length).toBe(creates + 2);
  expect(fixture.files.size).toBe(3);
  await page
    .getByRole("combobox", { name: "Theme", exact: true })
    .selectOption("dark");
  await page.screenshot({ path: `${out}/publication-complete-dark.png` });
  await axe(page);
  await page.getByRole("button", { name: "Done", exact: true }).click();
  const list = page
    .getByRole("region", { name: "Synced profiles" })
    .getByRole("listitem");
  await expect(list.first()).toContainText("Browser profile");
  await expect(list.first()).toContainText(
    "1 account · 3 settings · 3 files · Ready",
  );
  await page
    .getByRole("button", { name: "Find profiles", exact: true })
    .click();
  await expect(discoveryStatus(page)).toContainText(
    "Discovery complete: 3 files verified.",
  );
});

test("enrollment review, application, lost account reply and reviewed reconnect", async ({
  page,
}) => {
  const fixture = new DriveFixture();
  await connected(fixture);
  for (const op of publishedProfile(uuid(1), 10, "Laptop")) fixture.add(op);
  await fixture.install(page);
  await seedAccounts(page, ["existing@example.test"]);
  await page.goto("/");
  await openPreferences(page);
  await expect(discoveryStatus(page)).toContainText(
    "Discovery complete: 3 files verified.",
  );
  await page.evaluate(async () => {
    const path = "/src/provider.ts";
    const { GatewayRepository } = await import(path);
    const original = GatewayRepository.prototype.importAccount;
    let lost = true;
    GatewayRepository.prototype.importAccount = async function (
      ...args: any[]
    ) {
      const result = await original.apply(this, args);
      if (lost) {
        lost = false;
        throw new Error("The account reply was lost.");
      }
      return result;
    };
  });
  await page.getByRole("button", { name: "Use Laptop", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Use profile" });
  await expect(dialog).toContainText("Profile “Laptop”");
  await page
    .getByRole("button", { name: "Details for alex@example.test" })
    .click();
  await expect(dialog).toContainText("imap.example.test");
  await expect(
    page.getByRole("checkbox", { name: "Tooltips: off" }),
  ).toBeDisabled();
  await page.getByRole("checkbox", { name: "Preview lines: 3" }).uncheck();
  await page.screenshot({ path: `${out}/enrollment-review-light.png` });
  await axe(page);
  await page.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(dialog.getByRole("alert")).toContainText(
    "The account reply was lost.",
  );
  await page
    .getByRole("button", { name: "Retry enrollment", exact: true })
    .click();
  await expect(
    dialog.getByRole("status", { name: "Enrollment progress" }),
  ).toContainText("Applied 2 items");
  await page
    .getByRole("combobox", { name: "Theme", exact: true })
    .selectOption("light");
  await expect(
    dialog.getByRole("status", { name: "Enrollment progress" }),
  ).toContainText("Applied 2 items");
  await page.getByRole("button", { name: "Done", exact: true }).click();
  // The imported account exists once, without a password, and mail is untouched.
  const rows = page.locator(".account-connection", {
    hasText: "alex@example.test",
  });
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText(
    "Reconnect required: imported from a synced profile without a password",
  );
  await expect(
    page.locator(".account-connection", { hasText: "existing@example.test" }),
  ).toHaveCount(1);
  await page
    .getByRole("combobox", { name: "Theme", exact: true })
    .selectOption("dark");
  await capture(page, "Synced profiles", "enrollment-complete-dark");
  await axe(page);
  await page
    .getByRole("button", { name: "Reconnect alex@example.test", exact: true })
    .click();
  // The shared definition declares a separate SMTP password: both are entered
  // here, never imported from the profile.
  await page
    .getByRole("textbox", { name: "Incoming password" })
    .fill("synthetic-password");
  await page
    .getByRole("textbox", { name: "SMTP password" })
    .fill("synthetic-smtp-password");
  await page
    .getByRole("button", { name: "Verify and save account", exact: true })
    .click();
  await expect(rows.first()).toContainText("Connected in this tab");
  await page.reload();
  await openPreferences(page);
  await expect(
    page.locator(".account-connection", { hasText: "alex@example.test" }),
  ).toContainText("Reconnect to refresh or send");
});

test("first-setup onboarding offers opt-in, single-profile automatic enrollment and a picker", async ({
  page,
}) => {
  const fixture = new DriveFixture();
  await connected(fixture);
  await fixture.install(page);
  await page.goto("/");
  const offer = page.getByRole("region", { name: "Set up sync" });
  await expect(offer).toContainText("Sync accounts and settings");
  await page.screenshot({ path: `${out}/onboarding-optin-light.png` });
  await axe(page);
  await page.getByRole("button", { name: "Not now", exact: true }).click();
  await expect(offer).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole("heading", { name: "Inbox" })).toBeVisible();
  await expect(offer).toHaveCount(0);
});
test("a single existing profile is offered and enrolled automatically", async ({
  page,
}) => {
  const fixture = new DriveFixture();
  await connected(fixture);
  for (const op of publishedProfile(uuid(1), 10, "Laptop")) fixture.add(op);
  await fixture.install(page);
  await page.goto("/");
  const offer = page.getByRole("region", { name: "Set up sync" });
  await expect(offer).toContainText("Use “Laptop” on this browser?");
  await expect(offer).toContainText("1 account and 3 settings were found");
  await page
    .getByRole("button", { name: "Use this profile", exact: true })
    .click();
  await expect(offer).toHaveCount(0);
  await openPreferences(page);
  await expect(
    page.getByRole("status", { name: "Enrollment status" }),
  ).toContainText("applied: 3 items applied, 0 kept local");
  await expect(
    page.locator(".account-connection", { hasText: "alex@example.test" }),
  ).toContainText("Reconnect required");
  await expect(
    page.getByRole("combobox", { name: "Theme", exact: true }),
  ).toHaveValue("dark");
  await capture(page, "Synced profiles", "onboarding-enrolled-dark");
  await axe(page);
});
test("several profiles show a picker and never merge automatically", async ({
  page,
}) => {
  const fixture = new DriveFixture();
  await connected(fixture);
  for (const op of publishedProfile(uuid(1), 10, "Laptop")) fixture.add(op);
  for (const op of publishedProfile(uuid(2), 20, "Phone", "phone@example.test"))
    fixture.add(op);
  await fixture.install(page);
  await page.goto("/");
  const offer = page.getByRole("region", { name: "Set up sync" });
  await expect(offer).toContainText("Choose a profile");
  await expect(offer.getByRole("listitem")).toHaveCount(2);
  await page.screenshot({ path: `${out}/onboarding-picker-light.png` });
  await axe(page);
  await offer.getByRole("button", { name: "Use Phone", exact: true }).click();
  await expect(offer).toHaveCount(0);
  await openPreferences(page);
  await expect(
    page.getByRole("status", { name: "Enrollment status" }),
  ).toContainText("Profile “Phone” applied");
  await expect(
    page.locator(".account-connection", { hasText: "phone@example.test" }),
  ).toHaveCount(1);
  await expect(
    page.locator(".account-connection", { hasText: "alex@example.test" }),
  ).toHaveCount(0);
});
