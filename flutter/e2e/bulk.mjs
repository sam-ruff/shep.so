// Group action controls in the browser preview: Select mode, checkboxes,
// long-press ranges, captured Select all, the frozen review, progress with
// Pause/Resume, Undo and History, in light and dark. Real pointer input over
// Flutter's semantics tree; no app-state action API.
import { chromium } from "playwright";
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const out = path.join(root, "artifacts/flutter/web/bulk");
await mkdir(out, { recursive: true });
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 412, height: 892 } });
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
const externalRequests = [];
await page.route("**/*", async (route) => {
  const url = new URL(route.request().url());
  if (
    ["http:", "https:"].includes(url.protocol) &&
    !["127.0.0.1", "localhost"].includes(url.hostname)
  ) {
    externalRequests.push(url.origin + url.pathname);
    return route.abort();
  }
  return route.continue();
});
async function labels() {
  return page.locator("flt-semantics").allTextContents();
}
async function clickText(text) {
  await page
    .getByRole("button", { name: text, exact: true })
    .or(page.getByRole("menuitem", { name: text, exact: true }))
    .or(page.getByRole("tab", { name: text, exact: true }))
    .last()
    .click();
}
async function waitText(text, timeout = 30000) {
  await page.waitForFunction(
    (t) =>
      [
        ...document.querySelectorAll(
          "flt-semantics, [aria-label], [aria-live]",
        ),
      ].some((e) =>
        (
          (e.textContent || "") +
          " " +
          (e.getAttribute("aria-label") || "")
        ).includes(t),
      ),
    text,
    { timeout },
  );
}
async function hasText(text) {
  return page.evaluate(
    (t) =>
      [
        ...document.querySelectorAll(
          "flt-semantics, [aria-label], [aria-live]",
        ),
      ].some((e) =>
        (
          (e.textContent || "") +
          " " +
          (e.getAttribute("aria-label") || "")
        ).includes(t),
      ),
    text,
  );
}
async function capture(name) {
  await page.waitForTimeout(150); // Let the last frame paint before the capture.
  await page.screenshot({ path: path.join(out, `${name}.png`) });
}
async function openDropdown(label) {
  const box = await page
    .getByRole("button", { name: new RegExp(label) })
    .boundingBox();
  await page.mouse.click(box.x + box.width - 36, box.y + box.height / 2);
}
const scenarios = [];
try {
  await page.goto(process.env.SHEP_FLUTTER_URL ?? "http://127.0.0.1:5181");
  await page.waitForSelector("flt-semantics-placeholder", {
    state: "attached",
  });
  await page.evaluate(() =>
    document.querySelector("flt-semantics-placeholder").click(),
  );
  await waitText("A little room for good ideas");

  // Light: checkbox, long-press range, captured Select all and Clear.
  await clickText("Select");
  await waitText("No messages selected");
  await page
    .getByRole("checkbox", { name: "Select A little room for good ideas" })
    .click();
  await waitText("1 selected");
  // Semantics clicks are taps, so the range uses its visible menu control;
  // the host widget scenario covers the long-press gesture itself.
  await clickText("Actions for Coffee on Thursday?");
  await clickText("Select up to here");
  await waitText("3 selected");
  // A press on a selected row deselects it without opening the reader.
  await page
    .getByRole("group", { name: /^Unread, Your week, a little clearer, selected/ })
    .click({ position: { x: 180, y: 40 } });
  await waitText("2 selected");
  assert.equal(await hasText("Reply all"), false);
  await clickText("Select all");
  await waitText("All 130 selected");
  await capture("bulk-selection-light");
  await clickText("Clear");
  await waitText("No messages selected");
  await clickText("Select all");
  await waitText("All 130 selected");
  scenarios.push("select-checkbox-range-all-clear");

  // Frozen review with counts per account and folder, declined once.
  await clickText("Archive selected");
  await waitText("Archive 130 messages");
  await waitText("Personal · Inbox: 86");
  await waitText("Work · Inbox: 44");
  await capture("bulk-review-light");
  await clickText("Cancel");
  await page.waitForTimeout(300);
  assert.equal(await hasText("Archive 130 messages"), false);
  scenarios.push("review-counts-decline");

  // Approve: immediate paint, Pause/Resume while steps run, completion, Undo.
  await clickText("Select");
  await clickText("Select all");
  await waitText("All 130 selected");
  await clickText("Archive selected");
  await waitText("Archive 130 messages");
  await clickText("Archive");
  await waitText("Archiving, ");
  await waitText("All clear", 5000);
  await capture("bulk-progress-light");
  await clickText("Pause");
  await waitText("Paused");
  await capture("bulk-paused-light");
  await clickText("Resume");
  await waitText("Archived 130", 60000);
  await capture("bulk-complete-light");
  await clickText("Undo");
  await waitText("A little room for good ideas");
  await waitText("Undone: 130 restored", 60000);
  await clickText("Dismiss group notification");
  scenarios.push("approve-progress-pause-resume-undo");

  // Dark: a flag group, then History with its expanded items.
  await clickText("Preferences");
  await waitText("Swipe left");
  await openDropdown("Theme");
  await clickText("Dark");
  await waitText("Preferences saved");
  await clickText("Mail");
  await waitText("A little room for good ideas");
  await clickText("Select");
  await clickText("Select all");
  await waitText("All 130 selected");
  await capture("bulk-selection-dark");
  await clickText("Mark read selected");
  await waitText("Mark read 130 messages");
  await capture("bulk-review-dark");
  await clickText("Mark read");
  await waitText("Marked read 65, 65 skipped", 60000);
  await capture("bulk-complete-dark");
  await clickText("Open navigation menu");
  await waitText("Group History");
  await clickText("Group History");
  await waitText("Mark read 130 messages");
  await waitText("Archive 130 messages");
  await page
    .getByRole("button", { name: "Show messages of Mark read 130 messages" })
    .click();
  await waitText("Skipped · Already up to date");
  await capture("bulk-history-dark");
  const more = page.getByRole("button", {
    name: "Load next 50 messages",
    exact: true,
  });
  for (let i = 0; i < 40 && !(await more.count()); i++) {
    await page.mouse.move(210, 500);
    await page.mouse.wheel(0, 600);
    await page.waitForTimeout(80); // Let the real scroll paint its lazy children.
  }
  await more.click();
  for (let i = 0; i < 40 && !(await hasText("Bulk message 60")); i++) {
    await page.mouse.move(210, 500);
    await page.mouse.wheel(0, 600);
    await page.waitForTimeout(80);
  }
  assert.ok(await hasText("Bulk message 60"), "second item page loads");
  await page.mouse.wheel(0, -100000);
  await page.waitForTimeout(200);
  await clickText("Back");
  await waitText("A little room for good ideas");
  scenarios.push("dark-read-group-history-items");

  assert.deepEqual(errors, []);
  assert.deepEqual(
    externalRequests,
    [],
    "The fictional Flutter preview must not contact outside services",
  );
  await writeFile(
    path.join(out, "result.json"),
    JSON.stringify({ passed: true, scenarios, errors }, null, 2),
  );
  console.log("PASS: Flutter browser group selection, review, execution, Undo and History");
} catch (error) {
  await page.screenshot({ path: path.join(out, "failure.png") });
  await writeFile(path.join(out, "failure.html"), await page.content());
  await writeFile(
    path.join(out, "labels.json"),
    JSON.stringify(await labels(), null, 2),
  );
  throw error;
} finally {
  await browser.close();
}
