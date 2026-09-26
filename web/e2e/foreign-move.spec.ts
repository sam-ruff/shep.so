import { expect, test, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { seed, profile, subject } from "./mailbox-fixture";

// Each flow seeds 125 messages and reconnects two accounts through Preferences.
test.describe.configure({ timeout: 90000 });

// Two fictional IMAP accounts with folder catalogues; every server reply is a
// route fixture, so nothing leaves the browser.
const mailbox = (name: string) => ({
  name,
  delimiter: "/",
  encoding: "Utf8",
  selectable: true,
  no_inferiors: false,
  non_existent: false,
  role: null,
});
const catalogs: Record<string, string[]> = {
  work: ["INBOX", "Archive", "Projects/Archive", "Receipts"],
  personal: ["INBOX", "Archive", "Home.Plans"],
};
interface Wire {
  path: string;
  body: any;
}
async function setup(page: Page, protocols = { work: "Imap", personal: "Imap" }) {
  await seed(page);
  await page.evaluate(
    async ({ profile, catalogs, protocols }) => {
      const path = "/src/storage.ts",
        { BrowserStore } = await import(path),
        store = await BrowserStore.open(profile);
      const changes: any[] = [];
      for (const id of ["work", "personal"]) {
        const account = await store.get("accounts", id);
        const protocol = protocols[id as "work" | "personal"];
        changes.push(
          {
            store: "accounts",
            key: id,
            value: { ...account, protocol, port: protocol === "Imap" ? 993 : 995 },
          },
          {
            store: "folderCatalogs",
            key: id,
            value: {
              account: id,
              complete: true,
              mailboxes: catalogs[id].map((name) => ({
                name,
                delimiter: "/",
                encoding: "Utf8",
                selectable: true,
                no_inferiors: false,
                non_existent: false,
                role: null,
              })),
            },
          },
        );
      }
      // Server identities for the IMAP messages this spec moves.
      for (const i of [0, 2, 4]) {
        const id = `m00${i}`,
          mail = await store.get("mail", id);
        changes.push({
          store: "mail",
          key: id,
          value: { ...mail, core: { ...mail.core, remote_id: `7.${i + 1}` } },
        });
      }
      await store.commit(changes);
      store.close();
    },
    { profile, catalogs, protocols },
  );
  const wire: Wire[] = [];
  const control = { hold: undefined as Promise<void> | undefined };
  await page.route("**/api/capabilities", (route) =>
    route.fulfill({ json: { mail: true, endpoints: [], transfer: true } }),
  );
  await page.route("**/api/mail/probe", (route) =>
    route.fulfill({ json: { connected: true } }),
  );
  await page.route("**/api/mail/sync", (route) => {
    const { connection } = route.request().postDataJSON();
    const id = connection.account.id;
    const folders = catalogs[id] ?? ["INBOX"];
    return route.fulfill({
      contentType: "application/x-ndjson",
      body:
        [
          { kind: "folders", account: id, folders },
          { kind: "done", folders },
        ]
          .map((event) => JSON.stringify(event))
          .join("\n") + "\n",
    });
  });
  await page.route("**/api/mail/transfer", async (route) => {
    const body = route.request().postDataJSON();
    wire.push({ path: "transfer", body });
    await control.hold;
    return route.fulfill({
      json: { committed: true, remote_id: `9.${wire.length}` },
    });
  });
  await page.route("**/api/mail/transfer/finish", (route) => {
    wire.push({ path: "finish", body: route.request().postDataJSON() });
    return route.fulfill({ json: { committed: true } });
  });
  await page.route("**/api/mail/move", (route) => {
    wire.push({ path: "move", body: route.request().postDataJSON() });
    return route.fulfill({ json: { committed: true, remote_id: "8.1" } });
  });
  await page.route("**/api/mail/resolve-move", (route) =>
    route.fulfill({ json: { mail: route.request().postDataJSON().receipt.current } }),
  );
  await page.reload();
  await expect(
    page.getByRole("button", { name: subject(0), exact: true }),
  ).toBeVisible({ timeout: 15000 });
  for (const id of ["work", "personal"]) {
    if (protocols[id as "work" | "personal"] !== "Imap") continue;
    await page.getByRole("button", { name: "Preferences", exact: true }).click();
    await page
      .getByRole("button", { name: `Reconnect ${id}@example.test`, exact: true })
      .click();
    const dialog = page.getByRole("dialog", {
      name: `Reconnect ${id}@example.test`,
      exact: true,
    });
    await dialog
      .getByLabel("Incoming password", { exact: true })
      .fill("fixture-move-password");
    await dialog
      .getByRole("button", { name: "Verify and save account", exact: true })
      .click();
    await expect(dialog).toHaveCount(0);
  }
  return { wire, control };
}
async function enable(page: Page, foreign = true) {
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  const cross = page.getByRole("checkbox", {
    name: "Allow moving mail between accounts",
    exact: true,
  });
  const other = page.getByRole("checkbox", {
    name: "Search other accounts' folders when moving",
    exact: true,
  });
  await expect(other).toBeDisabled();
  await cross.check();
  if (foreign) await other.check();
  await expect(other).toBeChecked({ checked: foreign });
  if (foreign)
    await page
      .locator(".settings-card", { has: cross })
      .screenshot({ path: "../artifacts/web/foreign-move-preferences.png" });
  await page.getByRole("button", { name: "Mail", exact: true }).click();
}
async function chooser(page: Page, i = 0, shortcut = false) {
  await page.getByRole("button", { name: subject(i), exact: true }).click();
  // The phone layout hides the reader toolbar; M is the default Move key.
  if (shortcut) await page.keyboard.press("m");
  else
    await page
      .getByRole("region", { name: "Message reader" })
      .getByRole("button", { name: "Move", exact: true })
      .click();
  const dialog = page.getByRole("dialog", { name: "Move message", exact: true });
  await expect(dialog).toBeVisible();
  return dialog;
}
const rows = (dialog: ReturnType<Page["getByRole"]>) =>
  dialog.locator(".move-choice");
async function cached(page: Page, id: string) {
  return page.evaluate(
    async ({ profile, id }) => {
      const path = "/src/storage.ts",
        { BrowserStore } = await import(path),
        store = await BrowserStore.open(profile);
      const mail = await store.get("mail", id);
      store.close();
      return mail?.core;
    },
    { profile, id },
  );
}

test("other accounts' folders appear only while typing, rank after home matches and need confirmation", async ({
  page,
}) => {
  const { wire } = await setup(page);
  await enable(page);
  const dialog = await chooser(page);
  const input = dialog.getByLabel("Find a folder", { exact: true });
  await expect(input).toBeFocused();
  // An empty query lists only this account's folders, without badges.
  await expect(rows(dialog)).toHaveText([
    /Archive/,
    /Inbox/,
    /Projects\/Archive/,
    /Receipts/,
  ]);
  await expect(dialog.locator(".account-badge")).toHaveCount(0);
  await input.pressSequentially("archive");
  await expect(rows(dialog)).toHaveCount(3);
  await expect(rows(dialog).nth(0)).toHaveAccessibleName("Archive");
  await expect(rows(dialog).nth(1)).toHaveAccessibleName("Projects/Archive");
  await expect(rows(dialog).nth(2)).toHaveAccessibleName(
    "Archive in personal@example.test",
  );
  await expect(rows(dialog).nth(2).locator(".account-badge")).toHaveText(
    "personal@example.test",
  );
  await page.screenshot({ path: "../artifacts/web/foreign-move-chooser-light.png" });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await input.fill("plans");
  await expect(rows(dialog)).toHaveCount(1);
  await expect(rows(dialog).first()).toHaveAccessibleName(
    "Home.Plans in personal@example.test",
  );
  // Enter on a badged first row asks before moving; nothing is sent yet.
  await page.keyboard.press("Enter");
  const confirm = page.getByRole("dialog", {
    name: "Move to another account?",
    exact: true,
  });
  await expect(confirm).toContainText("Move to Home.Plans?");
  await expect(confirm.locator(".move-confirm .account-badge")).toHaveText(
    "personal@example.test",
  );
  await expect(confirm).toContainText(
    "Enter moves it, Escape returns to the folder list.",
  );
  await page.screenshot({ path: "../artifacts/web/foreign-move-confirm-light.png" });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  expect(wire).toEqual([]);
  // Escape returns to the list with the typed query and focus.
  await page.keyboard.press("Escape");
  await expect(dialog).toBeVisible();
  await expect(input).toHaveValue("plans");
  await expect(input).toBeFocused();
  await page.keyboard.press("n");
  await expect(input).toHaveValue("plansn");
  await input.fill("plans");
  await page.keyboard.press("Enter");
  await expect(confirm).toBeVisible();
  await page.keyboard.press("n");
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("plans");
  expect(wire).toEqual([]);
  await page.keyboard.press("Enter");
  await expect(confirm).toBeVisible();
  await page.keyboard.press("y");
  await expect(confirm).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: subject(0), exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("status", { name: "Move notification" }),
  ).toContainText("Moved 1 message to Home.Plans");
  await expect.poll(() => wire.map((w) => w.path)).toEqual(["transfer", "finish"]);
  expect(wire[0].body).toMatchObject({
    source: { account: { id: "work" } },
    destination: { account: { id: "personal" } },
    folder: "Home.Plans",
    mail: { account_id: "work", remote_id: "7.1", folder: "INBOX" },
  });
  expect(wire[1].body.mail).toMatchObject({ account_id: "work", remote_id: "7.1" });
  await expect
    .poll(() => cached(page, "m000"))
    .toMatchObject({ account_id: "personal", folder: "Home.Plans", remote_id: "9.1" });
});

test("mouse controls confirm, cancel back to the list, and Undo moves the message back", async ({
  page,
}) => {
  const { wire, control } = await setup(page);
  await enable(page);
  await page.emulateMedia({ colorScheme: "dark" });
  const dialog = await chooser(page, 2);
  const input = dialog.getByLabel("Find a folder", { exact: true });
  await input.fill("home");
  await dialog
    .getByRole("button", { name: "Home.Plans in personal@example.test", exact: true })
    .click();
  const confirm = page.getByRole("dialog", {
    name: "Move to another account?",
    exact: true,
  });
  await page.screenshot({ path: "../artifacts/web/foreign-move-confirm-dark.png" });
  await confirm.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(input).toHaveValue("home");
  await expect(input).toBeFocused();
  await page.screenshot({ path: "../artifacts/web/foreign-move-chooser-dark.png" });
  let release!: () => void;
  control.hold = new Promise<void>((resolve) => {
    release = resolve;
  });
  await dialog
    .getByRole("button", { name: "Home.Plans in personal@example.test", exact: true })
    .click();
  await confirm.getByRole("button", { name: "Move", exact: true }).click();
  // The row leaves at once while the other account is still uploading.
  await expect(
    page.getByRole("button", { name: subject(2), exact: true }),
  ).toHaveCount(0);
  const toast = page.getByRole("status", { name: "Move notification" });
  await expect(toast).toContainText("Moved 1 message to Home.Plans");
  release();
  await expect.poll(() => wire.map((w) => w.path)).toEqual(["transfer", "finish"]);
  await expect
    .poll(() => cached(page, "m002"))
    .toMatchObject({ account_id: "personal", folder: "Home.Plans" });
  control.hold = undefined;
  await toast.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(
    page.getByRole("button", { name: subject(2), exact: true }),
  ).toBeVisible();
  await expect
    .poll(() => wire.map((w) => w.path))
    .toEqual(["transfer", "finish", "transfer", "finish"]);
  expect(wire[2].body).toMatchObject({
    source: { account: { id: "personal" } },
    destination: { account: { id: "work" } },
    folder: "INBOX",
  });
  await expect
    .poll(() => cached(page, "m002"))
    .toMatchObject({ account_id: "work", folder: "INBOX" });
});

test("without the preference, or from a POP3 message, only this account's folders are offered", async ({
  page,
}) => {
  const { wire } = await setup(page, { work: "Imap", personal: "Imap" });
  await enable(page, false);
  let dialog = await chooser(page);
  await dialog.getByLabel("Find a folder", { exact: true }).fill("plans");
  await expect(rows(dialog)).toHaveCount(0);
  await expect(dialog).toContainText("No matching folders.");
  await page.keyboard.press("Enter");
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  // An explicit destination account moves at once and never adds badges.
  dialog = await chooser(page);
  await dialog
    .getByLabel("Destination account", { exact: true })
    .selectOption("personal");
  await dialog.getByLabel("Find a folder", { exact: true }).fill("plans");
  await expect(dialog.locator(".account-badge")).toHaveCount(0);
  await page.keyboard.press("Enter");
  await expect(dialog).toHaveCount(0);
  await expect.poll(() => wire.map((w) => w.path)).toEqual(["transfer", "finish"]);
  expect(wire[0].body.folder).toBe("Home.Plans");
});

test("badged rows and the confirmation fit a phone-width window", async ({
  page,
}) => {
  await setup(page);
  await enable(page);
  await page.setViewportSize({ width: 390, height: 844 });
  const dialog = await chooser(page, 0, true);
  await dialog.getByLabel("Find a folder", { exact: true }).fill("archive");
  await expect(rows(dialog)).toHaveCount(3);
  const overflow = await dialog.evaluate((d) => d.scrollWidth - d.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
  await page.screenshot({ path: "../artifacts/web/foreign-move-chooser-mobile.png" });
  await rows(dialog).nth(2).click();
  const confirm = page.getByRole("dialog", {
    name: "Move to another account?",
    exact: true,
  });
  await expect(confirm.getByRole("button", { name: "Move", exact: true })).toBeInViewport();
  await page.screenshot({ path: "../artifacts/web/foreign-move-confirm-mobile.png" });
});

test("a POP3 source never offers other accounts' folders", async ({ page }) => {
  await setup(page, { work: "Pop3", personal: "Imap" });
  await enable(page);
  const dialog = await chooser(page);
  await dialog.getByLabel("Find a folder", { exact: true }).fill("plans");
  await expect(rows(dialog)).toHaveCount(0);
  await expect(dialog).toContainText("No matching folders in any account.");
});

test("a group choice in another account opens the review naming that account", async ({
  page,
}) => {
  const { wire } = await setup(page);
  await enable(page);
  await page.setViewportSize({ width: 900, height: 640 });
  await page.getByRole("button", { name: "Select", exact: true }).click();
  for (const i of [0, 2, 4])
    await page
      .getByRole("checkbox", { name: `Select ${subject(i)}`, exact: true })
      .click();
  await page
    .getByRole("button", { name: "Move selected messages", exact: true })
    .click();
  const move = page.getByRole("dialog", {
    name: "Move selected messages",
    exact: true,
  });
  await move.getByLabel("Destination folder", { exact: true }).fill("plans");
  await expect(rows(move)).toHaveCount(1);
  await page.screenshot({ path: "../artifacts/web/foreign-move-group-compact.png" });
  await page.keyboard.press("Enter");
  const review = page.getByRole("dialog", {
    name: "Review group action",
    exact: true,
  });
  await expect(review.locator(".move-into .account-badge")).toHaveText(
    "personal@example.test",
  );
  const apply = review.getByRole("button", {
    name: "Move to Home.Plans 3 messages",
    exact: true,
  });
  await expect(apply).toBeFocused();
  await page.screenshot({ path: "../artifacts/web/foreign-move-review-compact.png" });
  expect(wire).toEqual([]);
  await page.keyboard.press("y");
  await expect(review).toHaveCount(0);
  await expect.poll(() => wire.filter((w) => w.path === "transfer").length).toBe(3);
  await expect.poll(() => wire.filter((w) => w.path === "finish").length).toBe(3);
  for (const id of ["m000", "m002", "m004"])
    await expect
      .poll(() => cached(page, id))
      .toMatchObject({ account_id: "personal", folder: "Home.Plans" });
});

test("a POP3 group picks from listed folders only and never needs typed text", async ({
  page,
}) => {
  const { wire } = await setup(page, { work: "Pop3", personal: "Imap" });
  await enable(page);
  await page.getByRole("button", { name: "Select", exact: true }).click();
  for (const i of [0, 2, 4])
    await page
      .getByRole("checkbox", { name: `Select ${subject(i)}`, exact: true })
      .click();
  await page
    .getByRole("button", { name: "Move selected messages", exact: true })
    .click();
  const move = page.getByRole("dialog", {
    name: "Move selected messages",
    exact: true,
  });
  const folder = move.getByLabel("Destination folder", { exact: true });
  await expect(rows(move).first()).toBeVisible();
  await expect(move.getByRole("button", { name: "Review move" })).toHaveCount(0);
  await expect(move.locator(".move-note")).toHaveText(
    "Each message stays in its original account.",
  );
  // An unknown name offers no row and Enter opens nothing.
  await folder.fill("Nowhere at all");
  await expect(rows(move)).toHaveCount(0);
  await page.keyboard.press("Enter");
  await expect(move).toBeVisible();
  const review = page.getByRole("dialog", {
    name: "Review group action",
    exact: true,
  });
  await expect(review).toHaveCount(0);
  await folder.fill("archive");
  await expect(rows(move).first()).toHaveAccessibleName("Archive");
  await expect(move.locator(".account-badge")).toHaveCount(0);
  await page.screenshot({ path: "../artifacts/web/foreign-move-group-pop3.png" });
  await page.keyboard.press("Enter");
  const apply = review.getByRole("button", {
    name: "Archive 3 messages",
    exact: true,
  });
  await expect(apply).toBeFocused();
  await page.keyboard.press("y");
  await expect(review).toHaveCount(0);
  for (const id of ["m000", "m002", "m004"])
    await expect
      .poll(() => cached(page, id))
      .toMatchObject({ account_id: "work", folder: "Archive" });
  expect(wire.filter((w) => w.path === "transfer")).toEqual([]);
});
