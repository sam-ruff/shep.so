import { expect, test, type Locator, type Page } from "@playwright/test";
import { seed, profile } from "./mailbox-fixture";

async function savedDraft(page: Page, subject: string) {
  return page.evaluate(
    async ({ profile, subject }) => {
      const path = "/src/storage.ts",
        { BrowserStore } = await import(path);
      const store = await BrowserStore.open(profile);
      try {
        const drafts: any[] = await store.all("drafts");
        const d = drafts.find((d) => d.subject === subject);
        return d ? { to: d.to, cc: d.cc, bcc: d.bcc } : undefined;
      } finally {
        store.close();
      }
    },
    { profile, subject },
  );
}
const chips = (editor: Locator, label: string) =>
  editor
    .getByRole("list", { name: `${label} recipients`, exact: true })
    .getByRole("listitem");
const entry = (editor: Locator, label: string) =>
  editor.getByRole("textbox", { name: label, exact: true });
async function paste(page: Page, field: Locator, text: string) {
  await page.evaluate((t) => navigator.clipboard.writeText(t), text);
  await field.focus();
  await page.keyboard.press("ControlOrMeta+v");
}

for (const compact of [false, true]) {
  test.describe(compact ? "compact dark" : "desktop light", () => {
    test.use({
      viewport: compact
        ? { width: 390, height: 844 }
        : { width: 1280, height: 900 },
      colorScheme: compact ? "dark" : "light",
      permissions: ["clipboard-read", "clipboard-write"],
    });
    test("recipients become chips through typing, paste, removal, reopening and undo", async ({
      page,
    }) => {
      const subject = compact ? "Compact chips" : "Desktop chips";
      await seed(page);
      const toggle = page.getByRole("button", {
        name: "Toggle navigation",
        exact: true,
      });
      const compose = page.getByRole("button", {
        name: "New message",
        exact: true,
      });
      if (!(await compose.isVisible())) await toggle.click();
      await compose.click();
      const editor = page.getByRole("dialog", {
        name: "New message",
        exact: true,
      });
      await editor
        .getByRole("combobox", { name: "From account", exact: true })
        .selectOption("work");
      await editor
        .getByRole("textbox", { name: "Subject", exact: true })
        .fill(subject);
      const to = entry(editor, "To"),
        cc = entry(editor, "Cc");

      // A separator finishes the typed address.
      await to.pressSequentially("alex@example.com,");
      await expect(chips(editor, "To")).toHaveText(["alex@example.com"]);
      await expect(to).toHaveValue("");

      // A copied column keeps its line breaks apart.
      await paste(page, to, "b@example.com\nc@example.com\n");
      await expect(chips(editor, "To")).toHaveText([
        "alex@example.com",
        "b@example.com",
      ]);
      await expect(to).toHaveValue("c@example.com");
      await page.keyboard.press("Enter");
      await expect(to).toHaveValue("");
      await expect(chips(editor, "To")).toHaveText([
        "alex@example.com",
        "b@example.com",
        "c@example.com",
      ]);

      // A semicolon list splits, and leaving the field finishes the rest.
      await paste(page, cc, "d@example.com;e@example.com");
      await expect(chips(editor, "Cc")).toHaveText(["d@example.com"]);
      await expect(cc).toHaveValue("e@example.com");
      await editor.getByRole("textbox", { name: "Subject", exact: true }).click();
      await expect(chips(editor, "Cc")).toHaveText([
        "d@example.com",
        "e@example.com",
      ]);
      await expect(cc).toHaveValue("");

      // Clicking empty space in the box focuses the entry.
      await editor.locator(".recipient-box").first().click({ position: { x: 3, y: 3 } });
      await expect(to).toBeFocused();

      await editor
        .getByRole("button", { name: "Remove b@example.com", exact: true })
        .click();
      await expect(chips(editor, "To")).toHaveText([
        "alex@example.com",
        "c@example.com",
      ]);

      // Backspace in an empty entry reopens the last chip for editing.
      await to.focus();
      await page.keyboard.press("Backspace");
      await expect(to).toHaveValue("c@example.com");
      await expect(chips(editor, "To")).toHaveText(["alex@example.com"]);

      await page.keyboard.press("ControlOrMeta+z");
      await expect(chips(editor, "To")).toHaveText([
        "alex@example.com",
        "c@example.com",
      ]);
      await expect(to).toHaveValue("");
      await page.keyboard.press("ControlOrMeta+z");
      await expect(chips(editor, "To")).toHaveText([
        "alex@example.com",
        "b@example.com",
        "c@example.com",
      ]);
      await page.keyboard.press("ControlOrMeta+Shift+z");
      await expect(chips(editor, "To")).toHaveText([
        "alex@example.com",
        "c@example.com",
      ]);

      // Addresses Send would reject are marked beyond colour.
      await to.pressSequentially("Not an address");
      await page.keyboard.press("Enter");
      const invalid = editor.locator(".recipient-chip.invalid");
      await expect(invalid).toHaveCount(1);
      await expect(invalid).toContainText("!");
      await expect(invalid).toContainText("(invalid address)");
      const width = compact ? "compact" : "desktop";
      for (const scheme of ["light", "dark"] as const) {
        await page.emulateMedia({ colorScheme: scheme });
        await page.screenshot({
          path: `../artifacts/web/recipient-chips-${width}-${scheme}.png`,
        });
      }
      await page.emulateMedia({ colorScheme: compact ? "dark" : "light" });
      await page.keyboard.press("ControlOrMeta+y");
      await expect(invalid).toHaveCount(1);
      await editor
        .getByRole("button", { name: "Remove Not an address", exact: true })
        .click();
      await expect(invalid).toHaveCount(0);

      await expect
        .poll(() => savedDraft(page, subject))
        .toEqual({
          to: "alex@example.com, c@example.com",
          cc: "d@example.com, e@example.com",
          bcc: "",
        });

      // Subject and the body keep their native undo.
      const subjectField = editor.getByRole("textbox", {
        name: "Subject",
        exact: true,
      });
      await subjectField.click();
      await subjectField.press("End");
      await subjectField.pressSequentially(" extra");
      await expect(subjectField).toHaveValue(`${subject} extra`);
      // Chromium may undo synthetic key presses one at a time.
      await page.keyboard.press("ControlOrMeta+z");
      await expect(subjectField).toHaveValue(new RegExp(`^${subject} ?e?x?t?r?$`));
      await expect(subjectField).not.toHaveValue(`${subject} extra`);
      await subjectField.fill(subject);
      const body = editor.getByRole("textbox", { name: "Message", exact: true });
      await body.click();
      await body.pressSequentially("Hello");
      await expect(body).toHaveValue("Hello");
      await page.keyboard.press("ControlOrMeta+z");
      await expect(body).toHaveValue(/^H?e?l?l?$/);
      await expect(chips(editor, "To")).toHaveCount(2);

      // The saved value reopens as chips after a restart.
      await editor.getByRole("button", { name: "Save draft", exact: true }).click();
      await expect(editor).toHaveCount(0);
      await page.reload();
      const drafts = page.getByRole("button", { name: "Drafts", exact: true });
      await expect(drafts.or(toggle).first()).toBeVisible();
      if (!(await drafts.isVisible())) await toggle.click();
      await drafts.click();
      await page.getByRole("button", { name: subject, exact: true }).click();
      const reopened = page.getByRole("dialog");
      await expect(chips(reopened, "To")).toHaveText([
        "alex@example.com",
        "c@example.com",
      ]);
      await expect(chips(reopened, "Cc")).toHaveText([
        "d@example.com",
        "e@example.com",
      ]);
      await expect(entry(reopened, "To")).toHaveValue("");
    });
  });
}
