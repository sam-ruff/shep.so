import { expect, test, type Page } from "@playwright/test";
import { seed, profile } from "./mailbox-fixture";

async function navigate(page: Page, name: string) {
  const target = page.getByRole("button", { name, exact: true });
  if (!(await target.isVisible()))
    await page
      .getByRole("button", { name: "Toggle navigation", exact: true })
      .click();
  await target.click();
}
const editor = (page: Page) =>
  page.getByRole("dialog", { name: "New message", exact: true });
const body = (page: Page) =>
  editor(page).getByRole("textbox", { name: "Message", exact: true });
const status = (page: Page) => editor(page).getByRole("status");

test("a new cached reply saves its first revision and reopens with exact text", async ({
  page,
}) => {
  await seed(page);
  await page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const store = await BrowserStore.open(profile);
    try {
      const original = await store.get("mail", "m000");
      await store.commit([{ store: "mail", key: "m000", value: { ...original, reply: {
        reply_to: [{ email: "sender@example.test", text: "sender@example.test" }],
        to: [{ email: "work@example.test", text: "work@example.test" }], cc: [],
        message_id: "<reply-fixture@example.test>", references: [],
      } } }]);
    } finally { store.close(); }
  }, profile);
  await page
    .getByRole("button", { name: "Selection letter 000", exact: true })
    .click();
  await page.getByRole("button", { name: "Reply", exact: true }).click();
  await body(page).fill("First exact reply text");
  await expect(status(page)).toHaveText("Saved on this browser");
  const subject = await editor(page)
    .getByRole("textbox", { name: "Subject", exact: true })
    .inputValue();
  await editor(page)
    .getByRole("button", { name: "Close", exact: true })
    .click();
  await page.reload();
  await navigate(page, "Drafts");
  await page.getByRole("button", { name: subject, exact: true }).click();
  await expect(body(page)).toHaveValue("First exact reply text");
  await expect(status(page)).toHaveText("Saved on this browser");
  const headers = await page.evaluate(async profile => {
    const path = "/src/storage.ts", { BrowserStore } = await import(path);
    const store = await BrowserStore.open(profile);
    try { const drafts = await store.all("drafts"); return { inReplyTo: drafts[0].inReplyTo, references: drafts[0].references }; }
    finally { store.close(); }
  }, profile);
  expect(headers).toEqual({ inReplyTo: "<reply-fixture@example.test>", references: ["<reply-fixture@example.test>"] });
});

for (const compact of [false, true]) {
  test.describe(
    compact ? "compact dark conflicts" : "desktop light conflicts",
    () => {
      test.use({
        viewport: compact
          ? { width: 390, height: 844 }
          : { width: 1280, height: 900 },
        colorScheme: compact ? "dark" : "light",
      });
      test("two editors review exact versions, retain stale choices and refresh on reopen", async ({
        page,
        context,
      }) => {
        await seed(page);
        await navigate(page, "New message");
        await editor(page)
          .getByRole("textbox", { name: "Subject", exact: true })
          .fill("Shared draft");
        await body(page).fill("Base text");
        await expect(status(page)).toHaveText("Saved on this browser");
        await editor(page)
          .getByLabel("Choose attachments", { exact: true })
          .setInputFiles({
            name: "shared.bin",
            mimeType: "application/octet-stream",
            buffer: Buffer.from([0, 255, 13, 10]),
          });
        await expect(
          editor(page).getByRole("button", {
            name: "Remove shared.bin",
            exact: true,
          }),
        ).toBeEnabled();
        const other = await context.newPage();
        await other.route("**/api/session", (route) =>
          route.fulfill({
            json: {
              email: "owner@example.test",
              user_id: profile,
              csrf: "X".repeat(43),
            },
          }),
        );
        await other.goto("/");
        await navigate(other, "Drafts");
        await other
          .getByRole("button", { name: "Shared draft", exact: true })
          .click();
        await expect(body(other)).toHaveValue("Base text");
        await body(page).fill("First editor saved text");
        await expect(status(page)).toHaveText("Saved on this browser");
        await body(other).fill("Second editor retained text");
        await expect(status(other)).toContainText("Not saved");
        await expect(
          editor(other).getByRole("button", {
            name: "Retry save",
            exact: true,
          }),
        ).toBeHidden();
        await body(other).fill("Second editor newest retained text");
        await other.evaluate(async () => {
          const path = "/src/storage.ts",
            { BrowserStore } = await import(path);
          const get = BrowserStore.prototype.get;
          let fail = true;
          BrowserStore.prototype.get = async function (
            store: string,
            key: string,
          ) {
            if (fail && store === "drafts") {
              fail = false;
              throw Error("Fixture saved draft read failed");
            }
            return get.call(this, store, key);
          };
        });
        await editor(other)
          .getByRole("button", { name: "Review saved draft", exact: true })
          .click();
        const review = other.getByRole("dialog", {
          name: "Review draft changes",
          exact: true,
        });
        await expect(review.getByRole("status")).toContainText(
          "saved draft read failed",
        );
        await expect(
          review.getByRole("button", { name: "Use saved text", exact: true }),
        ).toBeDisabled();
        await expect(body(other)).toHaveValue(
          "Second editor newest retained text",
        );
        await review
          .getByRole("button", { name: "Refresh saved version", exact: true })
          .click();
        await expect(
          review.getByRole("textbox", { name: "Saved text", exact: true }),
        ).toHaveValue(/First editor saved text/);
        await expect(
          review.getByRole("textbox", { name: "My text", exact: true }),
        ).toHaveValue(/Second editor newest retained text/);
        await other.screenshot({
          path: `../artifacts/web/draft-conflict-${compact ? "compact-dark" : "desktop-light"}.png`,
        });
        await body(page).fill("First editor changed after review");
        await expect(status(page)).toHaveText("Saved on this browser");
        await review
          .getByRole("button", { name: "Use saved text", exact: true })
          .click();
        await expect(review.getByRole("status")).toContainText(
          "conflicting edits",
        );
        await expect(body(other)).toHaveValue(
          "Second editor newest retained text",
        );
        await review
          .getByRole("button", { name: "Refresh saved version", exact: true })
          .click();
        await expect(
          review.getByRole("textbox", { name: "Saved text", exact: true }),
        ).toHaveValue(/First editor changed after review/);
        await review
          .getByRole("button", { name: "Save my text", exact: true })
          .click();
        await expect(review).toHaveCount(0);
        await expect(status(other)).toHaveText("Saved on this browser");
        await expect(body(other)).toHaveValue(
          "Second editor newest retained text",
        );
        await body(page).fill("Stale first editor choice");
        await expect(status(page)).toContainText("Not saved");
        await editor(page)
          .getByRole("button", { name: "Review saved draft", exact: true })
          .click();
        const firstReview = page.getByRole("dialog", {
          name: "Review draft changes",
          exact: true,
        });
        await firstReview
          .getByRole("button", { name: "Use saved text", exact: true })
          .click();
        await expect(body(page)).toHaveValue(
          "Second editor newest retained text",
        );
        await body(page).fill("Final text after checked adoption");
        await expect(status(page)).toHaveText("Saved on this browser");
        await editor(other)
          .getByRole("button", { name: "Close", exact: true })
          .click();
        await other
          .getByRole("button", { name: "Shared draft", exact: true })
          .click();
        await expect(body(other)).toHaveValue(
          "Final text after checked adoption",
        );
        await expect(
          editor(other).getByRole("button", {
            name: "Remove shared.bin",
            exact: true,
          }),
        ).toBeEnabled();
        await other.reload();
        await navigate(other, "Drafts");
        await other
          .getByRole("button", { name: "Shared draft", exact: true })
          .click();
        await expect(body(other)).toHaveValue(
          "Final text after checked adoption",
        );
        const stored = await other.evaluate(async (profile) => {
          const path = "/src/storage.ts",
            { BrowserStore } = await import(path);
          const store = await BrowserStore.open(profile);
          try {
            const files = await store.all("draftFiles");
            return {
              drafts: (await store.all("drafts")).length,
              files: files.length,
              bytes: [...new Uint8Array(await files[0].blob.arrayBuffer())],
              outgoing: (await store.all("outgoing")).length,
            };
          } finally {
            store.close();
          }
        }, profile);
        expect(stored).toEqual({
          drafts: 1,
          files: 1,
          bytes: [0, 255, 13, 10],
          outgoing: 0,
        });
      });
    },
  );
}
