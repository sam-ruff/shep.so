// Real controls, IndexedDB and production adapter against the Rust test router.
import { expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import assert from "node:assert/strict";
import path from "node:path";
export async function providerFlows(page, context, origin, output, session) {
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page
    .getByRole("button", { name: "Add mail account", exact: true })
    .click();
  const dialog = page.getByRole("dialog");
  await dialog
    .getByLabel("Account name", { exact: true })
    .fill("Mailbox fixture");
  await dialog
    .getByLabel("Email address", { exact: true })
    .fill("mailbox@example.test");
  await dialog.getByLabel("Incoming username", { exact: true }).fill("mailbox");
  await dialog
    .getByLabel("Incoming password", { exact: true })
    .fill("wrong-synthetic-password");
  await dialog.getByRole("button", { name: "Verify and save account" }).click();
  await expect(dialog.getByRole("status")).toContainText("Could not verify");
  await dialog
    .getByLabel("Incoming password", { exact: true })
    .fill("synthetic-password");
  for (const [width, height] of [
    [1440, 920],
    [900, 640],
  ]) {
    await page.setViewportSize({ width, height });
    const axe = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
      .analyze();
    assert.deepEqual(
      axe.violations,
      [],
      `Account dialog accessibility at ${width}`,
    );
    await page.screenshot({
      path: path.join(output, `account-setup-${width}.png`),
    });
  }
  await page.setViewportSize({ width: 1440, height: 920 });
  await dialog.getByRole("button", { name: "Verify and save account" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByText("Connected in this tab")).toBeVisible();
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  const row = page
    .locator(".mail-row")
    .filter({ hasText: "The beta transport fixture" });
  await expect(row).toHaveCount(1);
  await row
    .getByRole("button", { name: "The beta transport fixture", exact: true })
    .click();
  await expect(
    page
      .getByRole("region", { name: "Message reader" })
      .getByText(
        "This fictional message passed through the Rust mail API into browser storage.",
        { exact: true },
      ),
  ).toBeVisible();
  await Promise.all([
    page.waitForResponse(
      (r) =>
        r.url().endsWith("/api/mail/flags") &&
        r.request().postDataJSON().starred === true &&
        r.status() === 200,
    ),
    row
      .getByRole("button", {
        name: "Flag The beta transport fixture",
        exact: true,
      })
      .click(),
  ]);
  await expect(
    row.getByRole("button", {
      name: "Unflag The beta transport fixture",
      exact: true,
    }),
  ).toBeVisible();
  // Hold the real server acknowledgment after commit. Undo must update the UI
  // now, then use the receipt only after this response is delivered.
  let releaseMove, sawMove;
  const release = new Promise((resolve) => {
    releaseMove = resolve;
  });
  const entered = new Promise((resolve) => {
    sawMove = resolve;
  });
  let first = true;
  const heldMove = async (route) => {
    if (!first) return route.continue();
    first = false;
    const response = await route.fetch();
    sawMove();
    await release;
    await route.fulfill({ response });
  };
  await context.route(`${origin}/api/mail/move`, heldMove);
  const archive = () =>
    page
      .getByRole("region", { name: "Message reader" })
      .getByRole("button", { name: "Archive", exact: true });
  await archive().click();
  await entered;
  await expect(row).toHaveCount(0);
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(row).toHaveCount(1);
  await page.screenshot({ path: path.join(output, "imap-undo-pending.png") });
  const undone = page.waitForResponse(
    (r) =>
      r.url().endsWith("/api/mail/move") &&
      r.request().postDataJSON().folder === "INBOX" &&
      r.status() === 200,
  );
  releaseMove();
  await undone;
  await context.unroute(`${origin}/api/mail/move`, heldMove);
  await row
    .getByRole("button", { name: "The beta transport fixture", exact: true })
    .click();
  await Promise.all([
    page.waitForResponse(
      (r) => r.url().endsWith("/api/mail/move") && r.status() === 200,
    ),
    archive().click(),
  ]);
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Mail refreshed");
  await expect(row).toHaveCount(0);
  await Promise.all([
    page.waitForResponse(
      (r) =>
        r.url().endsWith("/api/mail/move") &&
        r.request().postDataJSON().folder === "INBOX" &&
        r.status() === 200,
    ),
    page.getByRole("button", { name: "Undo", exact: true }).click(),
  ]);
  await expect(row).toHaveCount(1);
  await page.screenshot({
    path: path.join(output, "imap-undo-after-refresh.png"),
  });
  // Cached Reply all must work before reconnecting after a real reload.
  await page.reload();
  await page
    .locator(".mail-row")
    .getByRole("button", { name: "The beta transport fixture", exact: true })
    .click();
  await page.getByRole("button", { name: "Reply all", exact: true }).click();
  await expect(dialog.getByLabel("To", { exact: true })).toHaveValue(
    "Support <support@example.test>, Peer <peer@example.test>",
  );
  await expect(dialog.getByLabel("Cc", { exact: true })).toHaveValue(
    "Copy <copy@example.test>",
  );
  await expect(dialog.getByLabel("Message", { exact: true })).toHaveValue(
    /On 06 Sep 2026/,
  );
  await dialog.getByLabel("Bcc", { exact: true }).fill("hidden@example.test");
  await dialog
    .getByLabel("Subject", { exact: true })
    .fill("Lost response fixture");
  await dialog
    .getByLabel("Message", { exact: true })
    .fill("Saved before SMTP begins.");
  const [picker] = await Promise.all([
    page.waitForEvent("filechooser"),
    dialog.getByRole("button", { name: "Attach files", exact: true }).click(),
  ]);
  await picker.setFiles([
    {
      name: "first.txt",
      mimeType: "text/plain",
      buffer: Buffer.from("Remove this attachment"),
    },
    {
      name: "binary.bin",
      mimeType: "application/octet-stream",
      buffer: Buffer.from([0, 255, 1, 13, 10]),
    },
  ]);
  await expect(
    dialog.getByRole("button", { name: "Remove first.txt", exact: true }),
  ).toBeEnabled();
  await expect(
    dialog.getByRole("button", { name: "Remove binary.bin", exact: true }),
  ).toBeVisible();
  await dialog.getByRole("button", { name: "Save draft", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await page.reload();
  await page.getByRole("button", { name: "Drafts", exact: true }).click();
  await page
    .getByRole("button", { name: "Lost response fixture", exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: "Remove first.txt", exact: true }),
  ).toBeEnabled();
  await dialog
    .getByRole("button", { name: "Remove first.txt", exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: "Remove first.txt", exact: true }),
  ).toHaveCount(0);
  await expect(dialog.getByLabel("Message", { exact: true })).toBeEnabled();
  // A later text autosave must not recreate the removed association.
  await dialog
    .getByLabel("Message", { exact: true })
    .fill("Saved before SMTP begins. ");
  await dialog
    .getByLabel("Message", { exact: true })
    .fill("Saved before SMTP begins.");
  for (const [width, height] of [
    [1440, 920],
    [900, 640],
  ]) {
    await page.setViewportSize({ width, height });
    await dialog
      .getByRole("button", { name: "Remove binary.bin", exact: true })
      .scrollIntoViewIfNeeded();
    const actions = await dialog
      .getByRole("button", { name: "Send", exact: true })
      .boundingBox();
    assert(
      actions && actions.y >= 0 && actions.y + actions.height <= height,
      "Send stays inside the viewport",
    );
    const axe = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
      .analyze();
    assert.deepEqual(
      axe.violations,
      [],
      `Reply attachment accessibility at ${width}`,
    );
    await page.screenshot({
      path: path.join(output, `reply-attachments-${width}.png`),
    });
  }
  await page.setViewportSize({ width: 1440, height: 920 });
  await dialog.getByRole("button", { name: "Save draft", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await page.reload();
  await expect(page.locator(".mail-row")).toHaveCount(1);
  await expect(
    page.locator(".mail-row").getByRole("button", {
      name: "Unflag The beta transport fixture",
      exact: true,
    }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page
    .getByRole("button", {
      name: "Reconnect mailbox@example.test",
      exact: true,
    })
    .click();
  await dialog
    .getByLabel("Incoming password", { exact: true })
    .fill("synthetic-password");
  await dialog.getByRole("button", { name: "Verify and save account" }).click();
  await expect(dialog).toHaveCount(0);
  await page.getByRole("button", { name: "Drafts", exact: true }).click();
  await page
    .getByRole("button", { name: "Lost response fixture", exact: true })
    .click();
  await expect(dialog.getByLabel("Message", { exact: true })).toHaveValue(
    "Saved before SMTP begins.",
  );
  await expect(
    dialog.getByRole("button", { name: "Remove first.txt", exact: true }),
  ).toHaveCount(0);
  await expect(
    dialog.getByRole("button", { name: "Remove binary.bin", exact: true }),
  ).toBeEnabled();
  const loseResponse = async (route) => {
    const submitted = route.request().postDataJSON();
    const saved = await page.evaluate(
      async ({ user, id }) => {
        const db = await new Promise((resolve, reject) => {
          const r = indexedDB.open(`shep.mail.v1.${user}`);
          r.onsuccess = () => resolve(r.result);
          r.onerror = reject;
        });
        return await new Promise((resolve, reject) => {
          const tx = db.transaction("outgoing");
          const r = tx.objectStore("outgoing").getAll();
          tx.oncomplete = () => {
            db.close();
            resolve(r.result.find((record) => record.id === id));
          };
          tx.onabort = reject;
        });
      },
      { user: session.user_id, id: submitted.id },
    );
    assert.equal(saved.state, "submitting");
    assert.deepEqual(
      saved.wire,
      submitted.wire,
      "Exact prepared MIME is durable before SMTP",
    );
    await route.fetch();
    await route.abort("connectionreset");
  };
  await context.route(`${origin}/api/mail/send`, loseResponse);
  await dialog.getByRole("button", { name: "Send", exact: true }).click();
  await expect(dialog.getByRole("status")).toContainText(
    "Delivery is not confirmed",
  );
  await expect(
    dialog.getByRole("button", { name: "Check delivery status" }),
  ).toBeVisible();
  await context.unroute(`${origin}/api/mail/send`, loseResponse);
  await page.screenshot({ path: path.join(output, "lost-send-response.png") });
  // The durable receipt is recovered without a password after a real reload.
  await page.reload();
  await page.getByRole("button", { name: "Drafts", exact: true }).click();
  await page
    .getByRole("button", { name: "Lost response fixture", exact: true })
    .click();
  await dialog.getByRole("button", { name: "Check delivery status" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByText("No saved drafts", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Sent", exact: true }).click();
  await page
    .locator(".mail-row")
    .getByRole("button", { name: "Lost response fixture", exact: true })
    .click();
  await expect(
    page
      .getByRole("region", { name: "Message reader" })
      .getByText("Saved before SMTP begins.", { exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: path.join(output, "local-sent-after-reload.png"),
  });
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.locator(".mail-row")).toHaveCount(1);
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page
    .getByRole("button", {
      name: "Reconnect mailbox@example.test",
      exact: true,
    })
    .click();
  await dialog
    .getByLabel("Incoming password", { exact: true })
    .fill("synthetic-password");
  await dialog.getByRole("button", { name: "Verify and save account" }).click();
  await expect(dialog).toHaveCount(0);
  await page.getByRole("button", { name: "New message", exact: true }).click();
  await dialog.getByLabel("To", { exact: true }).fill("recipient@example.test");
  await dialog
    .getByLabel("Subject", { exact: true })
    .fill("Uncertain delivery fixture");
  await dialog
    .getByLabel("Message", { exact: true })
    .fill("Delivery cannot be established.");
  const [uncertainPicker] = await Promise.all([
    page.waitForEvent("filechooser"),
    dialog.getByRole("button", { name: "Attach files", exact: true }).click(),
  ]);
  await uncertainPicker.setFiles({
    name: "review.bin",
    mimeType: "application/octet-stream",
    buffer: Buffer.from([0, 255, 1]),
  });
  await expect(
    dialog.getByRole("button", { name: "Remove review.bin", exact: true }),
  ).toBeEnabled();
  await dialog.getByRole("button", { name: "Send", exact: true }).click();
  await expect(dialog.getByRole("status")).toContainText(
    "Delivery remains unconfirmed",
  );
  await dialog.getByRole("button", { name: "Check delivery status" }).click();
  await expect(dialog.getByRole("status")).toContainText("not resent");
  await page.screenshot({ path: path.join(output, "uncertain-delivery.png") });
  await dialog.getByRole("button", { name: "Save draft", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  const outboxCard = (subject) =>
    dialog.locator(".settings-card").filter({
      has: page.getByRole("heading", { name: subject, exact: true }),
    });
  const openOutbox = () =>
    page.getByRole("button", { name: "Outbox", exact: true }).click();
  const reviewLabel =
    "I reviewed delivery; another send could create a duplicate";
  // Review controls remain disabled until the user acknowledges uncertainty.
  for (const [theme, width, height] of [
    ["light", 1440, 920],
    ["dark", 900, 640],
  ]) {
    await page.setViewportSize({ width, height });
    await page
      .getByRole("button", { name: "Preferences", exact: true })
      .click();
    await page.getByLabel("Theme", { exact: true }).selectOption(theme);
    await openOutbox();
    const uncertain = outboxCard("Uncertain delivery fixture");
    await expect(
      uncertain.getByRole("button", { name: "Return to drafts", exact: true }),
    ).toBeDisabled();
    await expect(
      uncertain.getByRole("button", { name: "Record as sent", exact: true }),
    ).toBeDisabled();
    const actionBox = await uncertain
      .getByRole("button", { name: "Return to drafts", exact: true })
      .boundingBox();
    assert.ok(
      actionBox && actionBox.y >= 0 && actionBox.y + actionBox.height <= height,
      "Recovery actions remain visible in the compact Outbox",
    );
    const scrollBox = await dialog.locator(".outbox-entries").boundingBox();
    const cardBox = await uncertain.boundingBox();
    assert.ok(
      scrollBox &&
        cardBox &&
        cardBox.y + cardBox.height <= scrollBox.y + scrollBox.height,
      "Recovery card is not clipped inside its scrolling container",
    );
    const closeBox = await dialog
      .getByRole("button", { name: "Close", exact: true })
      .boundingBox();
    assert.ok(
      closeBox && closeBox.y >= 0 && closeBox.y + closeBox.height <= height,
      "Close remains visible",
    );
    const axe = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
      .analyze();
    assert.deepEqual(axe.violations, [], `Outbox accessibility at ${width}`);
    await page.screenshot({
      path: path.join(output, `outbox-review-${theme}-${width}.png`),
    });
    await dialog.getByRole("button", { name: "Close", exact: true }).click();
  }
  await page.setViewportSize({ width: 1440, height: 920 });
  await openOutbox();
  await outboxCard("Lost response fixture")
    .getByRole("button", { name: "Keep local copy", exact: true })
    .click();
  await expect(outboxCard("Lost response fixture")).toHaveCount(0);
  const uncertain = outboxCard("Uncertain delivery fixture");
  await uncertain.getByLabel(reviewLabel, { exact: true }).check();
  await uncertain
    .getByRole("button", { name: "Return to drafts", exact: true })
    .click();
  await expect(
    dialog.getByText("No outgoing messages need attention.", { exact: true }),
  ).toBeVisible();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await page.getByRole("button", { name: "Drafts", exact: true }).click();
  await page
    .getByRole("button", { name: "Uncertain delivery fixture", exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: "Remove review.bin", exact: true }),
  ).toBeEnabled();
  await expect(dialog.getByLabel("Message", { exact: true })).toHaveValue(
    "Delivery cannot be established.",
  );
  await dialog
    .getByLabel("Subject", { exact: true })
    .fill("Uncertain delivery fixture reviewed");
  // A separate explicit Send is required for the recovered editable copy.
  await dialog.getByRole("button", { name: "Send", exact: true }).click();
  await expect(dialog.getByRole("status")).toContainText(
    "Delivery remains unconfirmed",
  );
  await dialog.getByRole("button", { name: "Save draft", exact: true }).click();
  await openOutbox();
  const reviewed = outboxCard("Uncertain delivery fixture reviewed");
  await reviewed.getByLabel(reviewLabel, { exact: true }).check();
  await reviewed
    .getByRole("button", { name: "Record as sent", exact: true })
    .click();
  await expect(
    dialog.getByText("No outgoing messages need attention.", { exact: true }),
  ).toBeVisible();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await page.getByRole("button", { name: "Sent", exact: true }).click();
  await page
    .locator(".mail-row")
    .getByRole("button", {
      name: "Uncertain delivery fixture reviewed",
      exact: true,
    })
    .click();
  await expect(
    page
      .getByRole("region", { name: "Message reader" })
      .getByText("Delivery cannot be established.", { exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: path.join(output, "outbox-reviewed-local-copy.png"),
  });

  await page.getByRole("button", { name: "New message", exact: true }).click();
  await dialog.getByLabel("To", { exact: true }).fill("recipient@example.test");
  await dialog
    .getByLabel("Subject", { exact: true })
    .fill("Rejected delivery fixture");
  await dialog
    .getByLabel("Message", { exact: true })
    .fill("Keep this draft after SMTP rejection.");
  await dialog.getByRole("button", { name: "Send", exact: true }).click();
  await expect(dialog.getByRole("status")).toContainText("not sent");
  await dialog.getByRole("button", { name: "Save draft", exact: true }).click();
  await openOutbox();
  const rejected = outboxCard("Rejected delivery fixture");
  await expect(rejected.getByRole("checkbox")).toHaveCount(0);
  await rejected
    .getByRole("button", { name: "Return to drafts", exact: true })
    .click();
  await expect(rejected).toHaveCount(0);
  await dialog.getByRole("button", { name: "Close", exact: true }).click();

  // Lose preparation after Rust has reserved exact bytes, before SMTP starts.
  const losePreparation = async (route) => {
    await route.fetch();
    await route.abort("connectionreset");
  };
  await context.route(`${origin}/api/mail/outgoing/prepare`, losePreparation);
  await page.getByRole("button", { name: "New message", exact: true }).click();
  await dialog.getByLabel("To", { exact: true }).fill("recipient@example.test");
  await dialog
    .getByLabel("Subject", { exact: true })
    .fill("Cancelled preparation fixture");
  await dialog
    .getByLabel("Message", { exact: true })
    .fill("No SMTP starts when preparation is lost.");
  await dialog.getByRole("button", { name: "Send", exact: true }).click();
  await expect(dialog.getByRole("status")).toContainText(
    "Delivery is not confirmed",
  );
  await context.unroute(`${origin}/api/mail/outgoing/prepare`, losePreparation);
  await dialog.getByRole("button", { name: "Save draft", exact: true }).click();
  await openOutbox();
  const cancelled = outboxCard("Cancelled preparation fixture");
  await cancelled
    .getByRole("button", { name: "Check delivery status", exact: true })
    .click();
  await expect(
    cancelled.getByText("Not yet submitted", { exact: true }),
  ).toBeVisible();
  await cancelled
    .getByRole("button", { name: "Return to drafts", exact: true })
    .click();
  await expect(cancelled).toHaveCount(0);
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await page.reload();
  await page.getByRole("button", { name: "Drafts", exact: true }).click();
  await expect(
    page.getByRole("button", {
      name: "Rejected delivery fixture",
      exact: true,
    }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Cancelled preparation fixture", exact: true })
    .click();
  await expect(dialog.getByLabel("Message", { exact: true })).toHaveValue(
    "No SMTP starts when preparation is lost.",
  );
  await expect(
    dialog.getByRole("button", { name: "Send", exact: true }),
  ).toBeVisible();
  await dialog.getByRole("button", { name: "Save draft", exact: true }).click();
  // Observation-only oracle: inspect stored values; never drive app state here.
  const persisted = await page.evaluate(async (user) => {
    const databases = await indexedDB.databases();
    const name = `shep.mail.v1.${user}`;
    if (!databases.some((db) => db.name === name))
      throw new Error("Missing scoped database");
    return new Promise((resolve, reject) => {
      const request = indexedDB.open(name);
      request.onsuccess = () => {
        const db = request.result;
        const tx = db.transaction([...db.objectStoreNames]);
        const values = [];
        for (const store of db.objectStoreNames) {
          const read = tx.objectStore(store).getAll();
          read.onsuccess = () => values.push(...read.result);
        }
        tx.oncomplete = () => {
          db.close();
          resolve(JSON.stringify(values));
        };
        tx.onabort = reject;
      };
    });
  }, session.user_id);
  assert.ok(persisted.includes("Uncertain delivery fixture"));
  assert.ok(!persisted.includes("synthetic-password"));
  assert.ok(!persisted.includes(session.csrf));
  return [
    "account-probe-failure-retry",
    "account-layout-axe-two-sizes",
    "streamed-mail-cache",
    "flag-ack-reload",
    "imap-queued-undo",
    "imap-undo-after-refresh",
    "draft-reopen",
    "cached-reply-all-offline",
    "attachment-picker-reopen-remove",
    "reply-attachment-axe-two-sizes",
    "exact-binary-MIME-reply-headers",
    "password-reconnect",
    "lost-send-response-reload-status",
    "exact-MIME-durable-before-SMTP",
    "local-Sent-after-reload",
    "uncertain-send-no-retry",
    "outbox-review-axe-light-dark",
    "outbox-local-Sent-choice",
    "outbox-reviewed-return-attachment-ownership",
    "outbox-explicit-new-send-and-manual-mark",
    "outbox-rejected-return",
    "outbox-lost-preparation-cancel-reopen",
    "browser-only-storage-no-secrets",
  ];
}
