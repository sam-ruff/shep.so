// Real controls, IndexedDB and production adapter against the Rust test router.
import { expect } from "@playwright/test";
import { messageFindFlow } from "./message-find-flow.mjs";
import AxeBuilder from "@axe-core/playwright";
import assert from "node:assert/strict";
import path from "node:path";
import { readFile } from "node:fs/promises";
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
  // A failed worker module request is visible and retryable. Mail bytes stay
  // in IndexedDB; cached attachment downloads then work with networking disabled.
  const wasmRoute = /\/assets\/shep_mail_content_bg[^/]*\.wasm$/;
  const failWasm = (route) => route.abort("failed");
  await context.route(wasmRoute, failWasm);
  await page
    .locator(".mail-row")
    .getByRole("button", { name: "Incoming files fixture", exact: true })
    .click();
  await expect(
    page.getByRole("alert").filter({ hasText: "cached attachment" }),
  ).toContainText("Could not read this cached attachment");
  await context.unroute(wasmRoute, failWasm);
  await page
    .getByRole("button", { name: "Reload attachments", exact: true })
    .click();
  const fileReader = page.getByRole("region", { name: "Message reader" });
  await expect(
    fileReader.getByRole("button", { name: "Save résumé.txt" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Retry formatted message", exact: true })
    .click();
  await expect(
    page
      .frameLocator('iframe[title="Formatted message"]')
      .getByText("Formatted cached files.", { exact: true }),
  ).toBeVisible();
  await expect(
    page.locator('iframe[title="Formatted message"]'),
  ).toHaveAttribute("sandbox", "allow-scripts");
  const popupReady = page.waitForEvent("popup");
  await fileReader.getByRole("button", { name: "Print", exact: true }).click();
  const printPreview = await popupReady;
  await expect(
    printPreview.getByRole("button", { name: "Print", exact: true }),
  ).toBeEnabled();
  await expect(
    printPreview.frameLocator("iframe").locator("body"),
  ).toContainText("Formatted cached files.");
  await printPreview.screenshot({
    path: path.join(output, "print-real-https.png"),
  });
  await printPreview.close();
  await page
    .getByRole("combobox", { name: "Message format", exact: true })
    .selectOption("Plain text");
  await expect(fileReader).not.toContainText("OBSOLETE-MIME-ALTERNATIVE");
  await expect(fileReader.getByRole("button", { name: /^Save / })).toHaveCount(
    3,
  );
  await expect(fileReader).not.toContainText("inline-logo.webp");
  await context.setOffline(true);
  try {
    await messageFindFlow(page);
    for (const [index, bytes] of [
      [0, [0, 255, 1, 13, 10]],
      [1, [0, 1, 2]],
    ]) {
      const [download] = await Promise.all([
        page.waitForEvent("download"),
        fileReader
          .getByRole("button", { name: "Save binary.bin" })
          .nth(index)
          .click(),
      ]);
      assert.equal(download.suggestedFilename(), "binary.bin");
      assert.deepEqual([...(await readFile(await download.path()))], bytes);
    }
    const [download] = await Promise.all([
      page.waitForEvent("download"),
      fileReader.getByRole("button", { name: "Save résumé.txt" }).click(),
    ]);
    assert.equal(download.suggestedFilename(), "résumé.txt");
    assert.equal(
      (await readFile(await download.path())).toString("utf8"),
      "Café",
    );
    await page.screenshot({
      path: path.join(output, "incoming-attachments-offline.png"),
    });
  } finally {
    await context.setOffline(false);
  }
  for (const [theme, width, height] of [
    ["light", 1440, 920],
    ["dark", 900, 640],
  ]) {
    await page
      .getByRole("button", { name: "Preferences", exact: true })
      .click();
    await page.getByLabel("Theme", { exact: true }).selectOption(theme);
    await page.getByRole("button", { name: "Mail", exact: true }).click();
    await page.setViewportSize({ width, height });
    await expect(
      fileReader.getByRole("button", { name: "Save résumé.txt" }),
    ).toBeVisible();
    const axe = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
      .analyze();
    assert.deepEqual(axe.violations, []);
    await page.screenshot({
      path: path.join(output, `incoming-attachments-${theme}-${width}.png`),
    });
  }
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page.getByLabel("Theme", { exact: true }).selectOption("light");
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await page.setViewportSize({ width: 1440, height: 920 });
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
  const cachedUnread = () =>
    page.evaluate(async (user) => {
      const db = await new Promise((resolve, reject) => {
        const opening = indexedDB.open(`shep.mail.v1.${user}`);
        opening.onsuccess = () => resolve(opening.result);
        opening.onerror = () => reject(opening.error);
      });
      try {
        return await new Promise((resolve, reject) => {
          const tx = db.transaction("mail");
          const rows = tx.objectStore("mail").getAll();
          tx.oncomplete = () =>
            resolve(
              rows.result.find(
                (m) => m.core.subject === "The beta transport fixture",
              )?.core.unread,
            );
          tx.onerror = () => reject(tx.error);
        });
      } finally {
        db.close();
      }
    }, session.user_id);
  await expect(row).toHaveClass(/unread/);
  assert.equal(await cachedUnread(), true);
  await Promise.all([
    page.waitForResponse(
      (r) =>
        r.url().endsWith("/api/mail/flags") &&
        r.request().postDataJSON().unread === false &&
        r.status() === 200,
    ),
    page.getByRole("button", { name: "Preferences", exact: true }).click(),
  ]);
  await expect.poll(cachedUnread).toBe(false);
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await row
    .getByRole("button", { name: "The beta transport fixture", exact: true })
    .click();
  await page.getByRole("button", { name: "Mark unread", exact: true }).click();
  await expect.poll(cachedUnread).toBe(true);
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  assert.equal(await cachedUnread(), true);
  await page.getByRole("button", { name: "Mail", exact: true }).click();
  await row
    .getByRole("button", { name: "The beta transport fixture", exact: true })
    .click();
  await page.screenshot({
    path: path.join(output, "read-on-leave-real-https.png"),
  });
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
  await expect(page.locator(".mail-row")).toHaveCount(2);
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
  await page
    .getByRole("button", {
      name: "Sent copies for mailbox@example.test",
      exact: true,
    })
    .click();
  await dialog
    .getByLabel("Sent-copy policy", { exact: true })
    .selectOption("Automatic");
  await dialog
    .getByLabel("Server Sent folder", { exact: true })
    .fill("Sent Mail");
  await dialog
    .getByRole("button", { name: "Save Sent preferences", exact: true })
    .click();
  await page.getByRole("button", { name: "Outbox", exact: true }).click();
  const deliveredCopy = () =>
    dialog.locator(".settings-card").filter({
      has: page.getByRole("heading", {
        name: "Lost response fixture",
        exact: true,
      }),
    });
  await deliveredCopy()
    .getByRole("button", { name: "Check server Sent", exact: true })
    .click();
  await expect(deliveredCopy()).toContainText(
    "No matching provider Sent copy found yet",
  );
  const loseCopyResponse = async (route) => {
    const submitted = route.request().postDataJSON();
    const copyId = new URL(route.request().url()).pathname.split("/").at(-2);
    const saved = await page.evaluate(
      async ({ user, copyId }) => {
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
            resolve(r.result.find((record) => record.sent?.copyId === copyId));
          };
          tx.onabort = reject;
        });
      },
      { user: session.user_id, copyId },
    );
    assert.equal(saved.sent.state, "copying");
    assert.equal(saved.sent.folder, "Sent Mail");
    assert.deepEqual(saved.sent.copyAccount, submitted.connection.account);
    assert.deepEqual(saved.wire, submitted.wire);
    await route.fetch();
    await route.abort("connectionreset");
  };
  await context.route(`${origin}/api/mail/sent/*/copy`, loseCopyResponse);
  await deliveredCopy()
    .getByRole("button", { name: "Save copy to server Sent", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toContainText(
    "Sent upload is not confirmed",
  );
  await context.unroute(`${origin}/api/mail/sent/*/copy`, loseCopyResponse);
  // Sync the acknowledged-on-server copy before repairing the lost receipt.
  // The duplicate stays separate until proof is durable in the browser journal.
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await page.getByRole("button", { name: "Sent", exact: true }).click();
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Mail refreshed");
  const sentRows = page
    .locator(".mail-row")
    .filter({ hasText: "Lost response fixture" });
  await expect(sentRows).toHaveCount(2);
  const localKey = await page
    .locator('.mail-row[data-id*="local-sent-"]')
    .getAttribute("data-id");
  assert.ok(localKey);
  const providerRow = page.locator('.mail-row[data-id$=":Sent Mail:91.4"]');
  const providerKey = await providerRow.getAttribute("data-id");
  await providerRow
    .getByRole("button", { name: "Lost response fixture", exact: true })
    .click();
  await Promise.all([
    page.waitForResponse(
      (r) => r.url().endsWith("/api/mail/flags") && r.status() === 200,
    ),
    providerRow
      .getByRole("button", { name: "Flag Lost response fixture", exact: true })
      .click(),
  ]);
  await expect(
    providerRow.getByRole("button", {
      name: "Unflag Lost response fixture",
      exact: true,
    }),
  ).toBeVisible();

  // A freshly opened tab has no passwords. Repair through its real controls;
  // the first tab retains its reader and a provider-ID Undo closure.
  const reopened = await context.newPage();
  await reopened.goto(`${origin}/app/`);
  await reopened.reload();
  await reopened.getByRole("button", { name: "Outbox", exact: true }).click();
  const reopenedDialog = reopened.getByRole("dialog");
  const recovered = reopenedDialog.locator(".settings-card").filter({
    has: reopened.getByRole("heading", {
      name: "Lost response fixture",
      exact: true,
    }),
  });
  await recovered
    .getByRole("button", { name: "Check server Sent", exact: true })
    .click();
  await expect(recovered).toContainText("Provider Sent copy acknowledged");
  await reopened.screenshot({
    path: path.join(output, "provider-sent-recovered-after-reload.png"),
  });
  await reopened.close();

  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Mail refreshed");
  await expect(sentRows).toHaveCount(1);
  await expect(sentRows).toHaveAttribute("data-id", localKey);
  await expect(sentRows).toHaveClass(/selected/);
  const reader = page.getByRole("region", { name: "Message reader" });
  await expect(
    reader.getByText("Saved before SMTP begins.", { exact: true }),
  ).toBeVisible();
  await expect(
    reader.getByRole("button", { name: "Unflag", exact: true }),
  ).toBeEnabled();
  const handover = await page.evaluate(
    async ({ user, localKey, providerKey }) => {
      const db = await new Promise((resolve, reject) => {
        const r = indexedDB.open(`shep.mail.v1.${user}`);
        r.onsuccess = () => resolve(r.result);
        r.onerror = reject;
      });
      return new Promise((resolve, reject) => {
        const tx = db.transaction(["mail", "mailAliases", "raw", "outgoing"]);
        const mail = tx.objectStore("mail").get(localKey),
          alias = tx.objectStore("mailAliases").get(providerKey),
          raw = tx.objectStore("raw").get(localKey),
          out = tx.objectStore("outgoing").getAll();
        tx.oncomplete = () => {
          db.close();
          resolve({
            mail: mail.result,
            alias: alias.result,
            raw: raw.result,
            out: out.result.find((r) => r.mail?.core.id === localKey),
          });
        };
        tx.onabort = reject;
      });
    },
    { user: session.user_id, localKey, providerKey },
  );
  assert.deepEqual(handover.alias, { alias: providerKey, target: localKey });
  assert.equal(handover.mail.core.remote_id, "91.4");
  assert.equal(handover.mail.core.folder, "Sent Mail");
  assert.equal(handover.mail.local, undefined);
  assert.equal(
    handover.raw,
    handover.out.wire.raw,
    "Original submitted MIME remains exact",
  );
  // Undo was created for the provider ID before the other tab adopted it.
  await Promise.all([
    page.waitForResponse(
      (r) =>
        r.url().endsWith("/api/mail/flags") &&
        r.request().postDataJSON().starred === false &&
        r.request().postDataJSON().mail.remote_id === "91.4" &&
        r.status() === 200,
    ),
    page.getByRole("button", { name: "Undo", exact: true }).click(),
  ]);
  await expect(
    reader.getByRole("button", { name: "Flag", exact: true }),
  ).toBeEnabled();
  await Promise.all([
    page.waitForResponse(
      (r) => r.url().endsWith("/api/mail/move") && r.status() === 200,
    ),
    reader.getByRole("button", { name: "Archive", exact: true }).click(),
  ]);
  await expect(sentRows).toHaveCount(0);
  await expect(
    reader.getByText("Saved before SMTP begins.", { exact: true }),
  ).toBeVisible();
  await Promise.all([
    page.waitForResponse(
      (r) =>
        r.url().endsWith("/api/mail/move") &&
        r.request().postDataJSON().folder === "Sent Mail" &&
        r.status() === 200,
    ),
    page.getByRole("button", { name: "Undo", exact: true }).click(),
  ]);
  await expect(sentRows).toHaveCount(1);
  await expect(sentRows).toHaveAttribute("data-id", localKey);
  for (const [theme, width, height] of [
    ["light", 1440, 920],
    ["dark", 900, 640],
  ]) {
    await page
      .getByRole("button", { name: "Preferences", exact: true })
      .click();
    await page.getByLabel("Theme", { exact: true }).selectOption(theme);
    await page.getByRole("button", { name: "Mail", exact: true }).click();
    await page.setViewportSize({ width, height });
    await expect(
      reader.getByText("Saved before SMTP begins.", { exact: true }),
    ).toBeVisible();
    const axe = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
      .analyze();
    assert.deepEqual(axe.violations, []);
    await page.screenshot({
      path: path.join(output, `sent-handover-${theme}-${width}.png`),
    });
  }
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page
    .getByRole("button", {
      name: "Sent copies for mailbox@example.test",
      exact: true,
    })
    .click();
  await expect(
    dialog.getByLabel("Server Sent folder", { exact: true }),
  ).toHaveValue("Sent Mail");
  await expect(
    dialog.getByLabel("Sent-copy policy", { exact: true }),
  ).toHaveValue("Automatic");
  for (const [width, height] of [
    [1440, 920],
    [900, 640],
  ]) {
    await page.setViewportSize({ width, height });
    const axe = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
      .analyze();
    assert.deepEqual(axe.violations, []);
    await page.screenshot({
      path: path.join(output, `provider-sent-preferences-${width}.png`),
    });
  }
  await page.setViewportSize({ width: 1440, height: 920 });
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
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
    await uncertain.scrollIntoViewIfNeeded();
    const actionBox = await uncertain
      .getByRole("button", { name: "Return to drafts", exact: true })
      .boundingBox();
    assert.ok(
      actionBox && actionBox.y >= 0 && actionBox.y + actionBox.height <= height,
      "Recovery actions remain visible in the compact Outbox",
    );
    await expect(async () => {
      const scrollBox = await dialog.locator(".outbox-entries").boundingBox();
      const cardBox = await uncertain.boundingBox();
      assert.ok(
        scrollBox &&
          cardBox &&
          cardBox.y >= scrollBox.y &&
          cardBox.y + cardBox.height <= scrollBox.y + scrollBox.height,
        `Recovery card is not clipped inside its scrolling container: ${JSON.stringify({ scrollBox, cardBox })}`,
      );
    }).toPass({ timeout: 2000 });
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
    reviewed.getByRole("button", {
      name: "Save copy to server Sent",
      exact: true,
    }),
  ).toBeEnabled();
  await reviewed
    .getByRole("button", { name: "Save copy to server Sent", exact: true })
    .click();
  await expect(reviewed).toContainText("Sent upload not confirmed");
  await expect(
    reviewed.getByRole("button", {
      name: "Save copy to server Sent",
      exact: true,
    }),
  ).toBeDisabled();
  for (const [theme, width, height] of [
    ["light", 1440, 920],
    ["dark", 900, 640],
  ]) {
    await dialog.getByRole("button", { name: "Close", exact: true }).click();
    await page
      .getByRole("button", { name: "Preferences", exact: true })
      .click();
    await page.getByLabel("Theme", { exact: true }).selectOption(theme);
    await page.setViewportSize({ width, height });
    await openOutbox();
    await reviewed
      .getByRole("button", { name: "Check server Sent", exact: true })
      .scrollIntoViewIfNeeded();
    const axe = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
      .analyze();
    assert.deepEqual(axe.violations, []);
    await page.screenshot({
      path: path.join(output, `provider-sent-review-${theme}-${width}.png`),
    });
  }
  await reviewed
    .getByRole("button", { name: "Check server Sent", exact: true })
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
  // Forward runs the production WASM worker under the real gateway CSP.
  await page.getByRole("button", { name: "Inbox", exact: true }).click();
  await page
    .locator(".mail-row")
    .getByRole("button", { name: "Incoming files fixture", exact: true })
    .click();
  await page.getByRole("button", { name: "Forward", exact: true }).click();
  await expect(
    page.getByRole("dialog", { name: "Forward message" }),
  ).toBeVisible();
  for (const label of ["To", "Cc", "Bcc"])
    await expect(dialog.getByLabel(label, { exact: true })).toHaveValue("");
  await expect(dialog.locator(".draft-file")).toHaveCount(4);
  const quote = await dialog
    .getByLabel("Message", { exact: true })
    .inputValue();
  assert.ok(quote.includes("Find fixture sentinel."));
  assert.ok(!quote.includes("OBSOLETE-MIME-ALTERNATIVE"));
  await dialog
    .getByLabel("Message", { exact: true })
    .fill("Forward through the real gateway.\n" + quote);
  await dialog.getByRole("button", { name: "Save draft", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await page.reload();
  await page.getByRole("button", { name: "Drafts", exact: true }).click();
  await page
    .getByRole("button", { name: "Fwd: Incoming files fixture", exact: true })
    .click();
  await expect(dialog.locator(".draft-file")).toHaveCount(4);
  await expect(dialog.getByLabel("Message", { exact: true })).toHaveValue(
    "Forward through the real gateway.\n" + quote,
  );
  await page.screenshot({
    path: path.join(output, "forward-real-https-reopened.png"),
  });
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
  assert.ok(persisted.includes("Formatted cached files."));
  assert.ok(!persisted.includes("synthetic-password"));
  assert.ok(!persisted.includes(session.csrf));
  const stale = await context.newPage();
  await stale.goto(origin + "/app/");
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page
    .getByRole("button", { name: "Remove mailbox@example.test", exact: true })
    .click();
  await expect(dialog.getByText(/cached messages/)).toBeVisible();
  await stale.getByRole("button", { name: "Drafts", exact: true }).click();
  await stale
    .getByRole("button", { name: "Rejected delivery fixture", exact: true })
    .click();
  await stale
    .getByRole("dialog")
    .getByLabel("Message", { exact: true })
    .fill("Draft changed while account removal was open.");
  await stale
    .getByRole("dialog")
    .getByRole("button", { name: "Save draft", exact: true })
    .click();
  const discard = dialog.getByLabel(
    "Discard unfinished delivery and move records",
  );
  if (await discard.isVisible()) await discard.check();
  await dialog
    .getByRole("button", { name: "Remove from browser", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toContainText("Local data changed");
  await dialog
    .getByRole("button", { name: "Reload removal counts", exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: "Reload removal counts" }),
  ).toBeEnabled();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(
    page.getByRole("button", {
      name: "Reconnect mailbox@example.test",
      exact: true,
    }),
  ).toBeVisible();
  await stale.getByRole("button", { name: "Preferences", exact: true }).click();
  await stale
    .getByRole("button", {
      name: "Reconnect mailbox@example.test",
      exact: true,
    })
    .click();
  await stale
    .getByRole("dialog")
    .getByLabel("Incoming password", { exact: true })
    .fill("synthetic-password");
  for (const [mode, width, height] of [
    ["dark", 900, 640],
    ["light", 1440, 920],
  ]) {
    await page.getByLabel("Theme", { exact: true }).selectOption(mode);
    await page.setViewportSize({ width, height });
    await page
      .getByRole("button", { name: "Remove mailbox@example.test", exact: true })
      .click();
    await expect(dialog.getByText(/cached messages/)).toBeVisible();
    assert.deepEqual(
      (
        await new AxeBuilder({ page })
          .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
          .analyze()
      ).violations,
      [],
    );
    await page.screenshot({
      path: path.join(output, `account-removal-${mode}-${width}.png`),
    });
    if (mode === "dark")
      await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  }
  if (await discard.isVisible()) await discard.check();
  await dialog
    .getByRole("button", { name: "Remove from browser", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByRole("button", {
      name: "Reconnect mailbox@example.test",
      exact: true,
    }),
  ).toHaveCount(0);
  await stale
    .getByRole("dialog")
    .getByRole("button", { name: "Verify and save account", exact: true })
    .click();
  await expect(stale.getByRole("dialog").getByRole("status")).toContainText(
    "account was removed",
  );
  await stale
    .getByRole("dialog")
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  await stale.getByRole("button", { name: "Mail", exact: true }).click();
  await stale.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(stale.locator(".mail-row")).toHaveCount(0);
  await stale.close();
  await page.getByRole("button", { name: "Drafts", exact: true }).click();
  await expect(
    page.getByRole("button", {
      name: "Rejected delivery fixture",
      exact: true,
    }),
  ).toHaveCount(0);
  await page.reload();
  await expect(page.locator(".mail-row")).toHaveCount(0);
  await page.getByRole("button", { name: "Drafts", exact: true }).click();
  await expect(
    page.getByRole("button", {
      name: "Rejected delivery fixture",
      exact: true,
    }),
  ).toHaveCount(0);
  return [
    "account-probe-failure-retry",
    "account-layout-axe-two-sizes",
    "streamed-mail-cache",
    "cached-attachment-worker-failure-retry",
    "formatted-reader-real-https-csp-worker-retry-and-plain-choice",
    "print-preview-real-https-beta-gate-csp-and-worker",
    "offline-exact-binary-duplicate-and-encoded-attachments",
    "incoming-files-light-dark-compact-axe",
    "read-on-leave-provider-ack-and-explicit-unread",
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
    "provider-Sent-preferences-reopen",
    "Sent-reservation-durable-before-APPEND",
    "Sent-lost-response-reopen-without-credentials",
    "Sent-reviewed-uncertain-copy-no-repeat",
    "Sent-light-dark-compact-axe",
    "Sent-provider-row-before-ack-repair",
    "Sent-cross-tab-alias-reader-and-Undo",
    "Sent-logical-folder-physical-move-Undo",
    "Sent-handover-light-dark-compact-axe",
    "browser-only-storage-no-secrets",
    "account-removal-stale-review-reload-cancel",
    "account-removal-light-dark-compact-axe",
    "account-removal-atomic-local-cleanup-reopen",
    "account-removal-stale-tab-reconnect-and-refresh",
    "offline-Find-Unicode-case-quotes-next-previous",
    "Forward-production-worker-CSP-files-and-restart",
  ];
}
