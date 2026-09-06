// Walkie Textie's Playwright + Flutter semantics approach, using real pointer
// input after activating the accessibility tree. No app-state action API.
import { chromium } from "playwright";
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const out = path.join(root, "artifacts/flutter/web");
await mkdir(out, { recursive: true });
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 412, height: 892 } });
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
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
async function openDropdown(label) {
  const box = await page
    .getByRole("button", { name: new RegExp(`^${label}`) })
    .boundingBox();
  await page.mouse.click(box.x + box.width - 36, box.y + box.height / 2);
}
async function waitText(text) {
  await page.waitForFunction(
    (t) =>
      [...document.querySelectorAll("flt-semantics")].some((e) =>
        (
          (e.textContent || "") +
          " " +
          (e.getAttribute("aria-label") || "")
        ).includes(t),
      ),
    text,
  );
}
try {
  await page.goto(process.env.SHEP_FLUTTER_URL ?? "http://127.0.0.1:5181");
  await page.waitForSelector("flt-semantics-placeholder", {
    state: "attached",
  });
  await page.evaluate(() =>
    document.querySelector("flt-semantics-placeholder").click(),
  );
  await waitText("A little room for good ideas");
  await page.screenshot({ path: path.join(out, "inbox-light.png") });
  const row = page.getByRole("group", {
    name: /^Unread, A little room for good ideas/,
  });
  const box = await row.boundingBox();
  await page.mouse.move(380, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(35, box.y + box.height / 2, { steps: 20 });
  await page.screenshot({ path: path.join(out, "swipe-archive-icon.png") });
  await page.mouse.up();
  await waitText("Moved to Archive");
  await clickText("Undo");
  await waitText("A little room for good ideas");
  await clickText("Preferences");
  await waitText("Swipe left");
  await openDropdown("Theme");
  await clickText("Dark");
  await waitText("Preferences saved");
  await page.screenshot({ path: path.join(out, "preferences-dark.png") });
  await clickText("Mail");
  await waitText("A little room for good ideas");
  await page.screenshot({ path: path.join(out, "inbox-dark.png") });
  await clickText("Calendar");
  await waitText("September 2026");
  await page.screenshot({ path: path.join(out, "calendar-dark.png") });
  assert.deepEqual(errors, []);
  await writeFile(
    path.join(out, "result.json"),
    JSON.stringify(
      {
        passed: true,
        scenarios: ["inbox", "swipe-archive", "undo", "appearance", "calendar"],
        errors,
      },
      null,
      2,
    ),
  );
  console.log("PASS: Flutter browser inbox, real swipe, undo, theme, calendar");
} catch (error) {
  await page.screenshot({ path: path.join(out, "failure.png") });
  await writeFile(
    path.join(out, "labels.json"),
    JSON.stringify(await labels(), null, 2),
  );
  throw error;
} finally {
  await browser.close();
}
