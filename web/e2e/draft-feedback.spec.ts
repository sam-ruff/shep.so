import { expect, test, type Page } from "@playwright/test";
import { seed, profile } from "./mailbox-fixture";

async function navigate(page: Page, name: string) {
  const button = page.getByRole("button", { name, exact: true });
  if (!(await button.isVisible()))
    await page
      .getByRole("button", { name: "Toggle navigation", exact: true })
      .click();
  await button.click();
}
async function compose(page: Page, subject: string) {
  await navigate(page, "New message");
  const editor = page.getByRole("dialog", { name: "New message", exact: true });
  await editor
    .getByRole("textbox", { name: "Subject", exact: true })
    .fill(subject);
  return editor;
}
async function saved(page: Page) {
  return page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path);
    const store = await BrowserStore.open(profile);
    try {
      return await store.snapshot(["drafts", "draftFiles"]);
    } finally {
      store.close();
    }
  }, profile);
}

for (const compact of [false, true]) {
  test.describe(compact ? "compact dark draft" : "desktop light draft", () => {
    test.use({
      viewport: compact
        ? { width: 390, height: 844 }
        : { width: 1280, height: 900 },
      colorScheme: compact ? "dark" : "light",
    });
    test("late failure retains newer text through navigation and Retry survives restart", async ({
      page,
    }) => {
      await seed(page);
      await page.evaluate(async () => {
        const path = "/src/storage.ts",
          { BrowserStore } = await import(path);
        const commit = BrowserStore.prototype.commit;
        let first = true;
        const fixture = window as any;
        fixture.failDraft = true;
        BrowserStore.prototype.commit = async function (changes: any[]) {
          if (
            changes.some(
              (c) =>
                c.store === "drafts" && c.value?.subject === "Retained draft",
            ) &&
            fixture.failDraft
          ) {
            if (first) {
              first = false;
              fixture.draftHeld = true;
              await new Promise<void>((resolve) => {
                fixture.releaseDraft = resolve;
              });
            }
            throw Error("Fixture draft storage unavailable");
          }
          return commit.call(this, changes);
        };
      });
      const editor = await compose(page, "Retained draft");
      await editor
        .getByRole("textbox", { name: "Message", exact: true })
        .fill("Older text");
      await expect
        .poll(() => page.evaluate(() => (window as any).draftHeld))
        .toBe(true);
      await editor
        .getByRole("textbox", { name: "Message", exact: true })
        .fill("Newest exact text after held write");
      await page.evaluate(() => (window as any).releaseDraft());
      await expect(editor.getByRole("status")).toContainText("Not saved");
      await expect(
        editor.getByRole("button", { name: "Retry save", exact: true }),
      ).toBeEnabled();
      await page.screenshot({
        path: `../artifacts/web/draft-error-${compact ? "compact-dark" : "desktop-light"}.png`,
      });
      const leaving = page.waitForEvent("dialog");
      await page.evaluate(() => {
        setTimeout(() => location.reload(), 0);
      });
      const warning = await leaving;
      expect(warning.type()).toBe("beforeunload");
      await warning.dismiss();
      await expect(
        editor.getByRole("textbox", { name: "Message", exact: true }),
      ).toHaveValue("Newest exact text after held write");
      await editor.getByRole("button", { name: "Close", exact: true }).click();
      let signouts = 0;
      await page.route("**/api/logout", (route) => {
        signouts++;
        return route.fulfill({ status: 204 });
      });
      await navigate(page, "Sign out");
      await expect(
        page.getByText(
          "Some drafts are not saved. Open Drafts and retry before signing out.",
          { exact: true },
        ),
      ).toBeVisible();
      expect(signouts).toBe(0);
      await navigate(page, "Preferences");
      await expect(
        page.getByRole("heading", { name: "Preferences", exact: true }),
      ).toBeVisible();
      await navigate(page, "Mail");
      await navigate(page, "Drafts");
      await expect(
        page.locator(".draft-row").filter({ hasText: "Retained draft" }),
      ).toContainText("Not saved");
      await page
        .getByRole("button", { name: "Retained draft", exact: true })
        .click();
      await expect(
        editor.getByRole("textbox", { name: "Message", exact: true }),
      ).toHaveValue("Newest exact text after held write");
      await page.evaluate(() => {
        (window as any).failDraft = false;
      });
      await editor
        .getByRole("button", { name: "Retry save", exact: true })
        .click();
      await expect(editor.getByRole("status")).toHaveText(
        "Saved on this browser",
      );
      await expect(
        editor.getByRole("button", { name: "Retry save", exact: true }),
      ).toBeHidden();
      await expect
        .poll(async () => (await saved(page)).drafts[0]?.body)
        .toBe("Newest exact text after held write");
      await page.reload();
      await navigate(page, "Drafts");
      await page
        .getByRole("button", { name: "Retained draft", exact: true })
        .click();
      await expect(
        editor.getByRole("textbox", { name: "Message", exact: true }),
      ).toHaveValue("Newest exact text after held write");
      await expect(editor.getByRole("status")).toHaveText(
        "Saved on this browser",
      );
    });
  });
}

test("closing before a failed autosave retains its text and a held Drafts press through the late error", async ({
  page,
}) => {
  await seed(page);
  await page.evaluate(async () => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path);
    const commit = BrowserStore.prototype.commit;
    let first = true;
    BrowserStore.prototype.commit = async function (changes: any[]) {
      if (
        first &&
        changes.some(
          (c) => c.store === "drafts" && c.value?.subject === "Parked draft",
        )
      ) {
        first = false;
        await new Promise<void>((resolve) => {
          (window as any).releaseParked = resolve;
        });
        throw Error("Fixture parked draft failure");
      }
      return commit.call(this, changes);
    };
  });
  const editor = await compose(page, "Parked draft");
  await editor
    .getByRole("textbox", { name: "Message", exact: true })
    .fill("Text retained after closing before acknowledgement");
  await expect
    .poll(() => page.evaluate(() => typeof (window as any).releaseParked))
    .toBe("function");
  await editor.getByRole("button", { name: "Close", exact: true }).click();
  await expect(editor).toHaveCount(0);
  await navigate(page, "Drafts");
  const row = page.locator(".draft-row").filter({ hasText: "Parked draft" });
  await expect(row).toContainText("Saving…");
  const target = row.getByRole("button", { name: "Parked draft", exact: true });
  const bounds = await target.boundingBox();
  expect(bounds).not.toBeNull();
  await page.mouse.move(
    bounds!.x + bounds!.width / 2,
    bounds!.y + bounds!.height / 2,
  );
  await page.mouse.down();
  await page.evaluate(() => (window as any).releaseParked());
  await expect(row).toContainText("Not saved");
  await page.mouse.up();
  await expect(editor).toBeVisible();
  await expect(
    editor.getByRole("textbox", { name: "Message", exact: true }),
  ).toHaveValue("Text retained after closing before acknowledgement");
  await expect(editor.getByRole("status")).toContainText(
    "parked draft failure",
  );
  await editor.getByRole("button", { name: "Retry save", exact: true }).click();
  await expect(editor.getByRole("status")).toHaveText("Saved on this browser");
});

test("attachment admission lost reply retries the same bytes once while newer text remains editable", async ({
  page,
}) => {
  await seed(page);
  const editor = await compose(page, "Attachment recovery");
  await editor
    .getByRole("textbox", { name: "Message", exact: true })
    .fill("Before attachment");
  await expect(editor.getByRole("status")).toHaveText("Saved on this browser");
  await page.evaluate(async () => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path);
    const commit = BrowserStore.prototype.commit;
    let fail = true;
    BrowserStore.prototype.commit = async function (changes: any[]) {
      await commit.call(this, changes);
      if (fail && changes.some((c) => c.store === "draftFiles" && c.value)) {
        fail = false;
        throw Error("Fixture attachment receipt lost");
      }
    };
  });
  await editor.getByLabel("Choose attachments", { exact: true }).setInputFiles({
    name: "exact.bin",
    mimeType: "application/octet-stream",
    buffer: Buffer.from([0, 255, 1, 13, 10]),
  });
  await expect(editor.getByRole("status")).toContainText(
    "attachment receipt lost",
  );
  await editor
    .getByRole("textbox", { name: "Message", exact: true })
    .fill("Newer text with retained attachment");
  await expect
    .poll(async () => (await saved(page)).drafts[0]?.body)
    .toBe("Newer text with retained attachment");
  await expect(editor.getByRole("status")).toContainText(
    "attachment receipt lost",
  );
  await editor.getByRole("button", { name: "Retry save", exact: true }).click();
  await expect(editor.getByRole("status")).toHaveText("Saved on this browser");
  await expect(
    editor.getByRole("button", { name: "Remove exact.bin", exact: true }),
  ).toBeEnabled();
  await expect.poll(async () => (await saved(page)).draftFiles.length).toBe(1);
  const bytes = await page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path);
    const store = await BrowserStore.open(profile);
    const files = await store.all("draftFiles");
    store.close();
    return [...new Uint8Array(await files[0].blob.arrayBuffer())];
  }, profile);
  expect(bytes).toEqual([0, 255, 1, 13, 10]);
  await editor
    .getByRole("button", { name: "Remove exact.bin", exact: true })
    .scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "../artifacts/web/draft-attachment-recovered.png",
  });
  await page.reload();
  await page.getByRole("button", { name: "Drafts", exact: true }).click();
  await page
    .getByRole("button", { name: "Attachment recovery", exact: true })
    .click();
  await expect(
    editor.getByRole("textbox", { name: "Message", exact: true }),
  ).toHaveValue("Newer text with retained attachment");
  await expect(
    editor.getByRole("button", { name: "Remove exact.bin", exact: true }),
  ).toBeEnabled();
  await page.evaluate(async () => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path);
    const commit = BrowserStore.prototype.commit;
    let fail = true;
    BrowserStore.prototype.commit = async function (changes: any[]) {
      await commit.call(this, changes);
      if (fail && changes.some((c) => c.store === "draftFiles" && !c.value)) {
        fail = false;
        throw Error("Fixture removal receipt lost");
      }
    };
  });
  await editor
    .getByRole("button", { name: "Remove exact.bin", exact: true })
    .click();
  await expect(editor.getByRole("status")).toContainText(
    "removal receipt lost",
  );
  await editor.getByRole("button", { name: "Retry save", exact: true }).click();
  await expect(editor.getByRole("status")).toHaveText("Saved on this browser");
  await expect(
    editor.getByRole("button", { name: "Remove exact.bin", exact: true }),
  ).toHaveCount(0);
  expect((await saved(page)).draftFiles).toHaveLength(0);
  await editor
    .getByLabel("Choose attachments", { exact: true })
    .setInputFiles(
      Array.from({ length: 33 }, (_, index) => ({
        name: `file-${index}.txt`,
        mimeType: "text/plain",
        buffer: Buffer.from("x"),
      })),
    );
  await expect(editor.getByRole("status")).toContainText("at most 32 files");
  await editor
    .getByRole("button", { name: "Use saved attachments", exact: true })
    .click();
  await expect(editor.getByRole("status")).toHaveText("Saved on this browser");
  await expect(
    editor.getByRole("button", { name: "Attach files", exact: true }),
  ).toBeEnabled();
  await expect(
    editor.getByRole("textbox", { name: "Message", exact: true }),
  ).toHaveValue("Newer text with retained attachment");
  expect((await saved(page)).draftFiles).toHaveLength(0);
});
