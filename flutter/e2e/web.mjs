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
async function openDropdown(label) {
  const box = await page
    .getByRole("button", { name: new RegExp(label) })
    .boundingBox();
  await page.mouse.click(box.x + box.width - 36, box.y + box.height / 2);
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
async function enterFind(value) {
  // Flutter can expose an inactive, full-viewport semantics input after the
  // case button takes focus. Target the painted field beside Close Find, then
  // send real keyboard input instead of mutating that proxy's DOM value.
  const close = await page
    .getByRole("button", { name: "Close Find", exact: true })
    .boundingBox();
  assert.ok(close);
  await page.mouse.click(close.x / 2, close.y + close.height / 2);
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.type(value);
}
async function scrollToGoogle(control) {
  for (let i = 0; i < 12; i++) {
    if (await control.count()) {
      const box = await control.boundingBox();
      if (box && box.y > 100 && box.y + box.height < 720) return;
    }
    await page.mouse.move(210, 430);
    await page.mouse.wheel(0, 340);
    await page.waitForTimeout(80); // Let the real scroll paint its lazy children.
  }
  throw new Error("Google control did not become visible");
}
async function capture(name) {
  await page.waitForTimeout(150); // Let the last frame paint before the capture.
  await page.screenshot({ path: path.join(out, `${name}.png`) });
}
// Visual parity captures reviewed against the desktop client: drawer, reader,
// composer and the empty search state in the current colour scheme.
async function visualParity(scheme) {
  await clickText("Open navigation menu");
  await waitText("New message");
  await capture(`drawer-${scheme}`);
  await page.mouse.click(390, 600);
  await waitText("A little room for good ideas");
  await page
    .getByRole("group", { name: /A little room for good ideas/ })
    .click();
  await waitText("Reply all");
  await capture(`reader-${scheme}`);
  await clickText("Back");
  await page
    .getByRole("group", { name: /^Read, A little room for good ideas/ })
    .waitFor();
  await clickText("Actions for A little room for good ideas");
  await clickText("Mark unread");
  await page
    .getByRole("group", { name: /^Unread, A little room for good ideas/ })
    .waitFor();
  await clickText("New message");
  await waitText("Save and close");
  await capture(`composer-${scheme}`);
  await clickText("Save and close");
  await waitText("A little room for good ideas");
  await clickText("Search");
  const search = page.getByRole("textbox").last();
  await search.click();
  // Flutter attaches its semantics input a frame after focus; real keyboard
  // input can land before it, so retry until the filtered list reacts.
  for (let attempt = 0; ; attempt++) {
    await page.waitForTimeout(150);
    await page.keyboard.type("zzzz");
    try {
      await waitText("No matching mail", 4000);
      break;
    } catch (error) {
      if (attempt === 2) throw error;
      await search.click();
    }
  }
  await capture(`empty-${scheme}`);
  await clickText("Clear search");
  await clickText("Search");
  await waitText("A little room for good ideas");
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
  await visualParity("light");
  const row = page.getByRole("group", {
    name: /^Unread, A little room for good ideas/,
  });
  const box = await row.boundingBox();
  await page.mouse.move(380, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(35, box.y + box.height / 2, { steps: 20 });
  await page.screenshot({ path: path.join(out, "swipe-archive-icon.png") });
  await page.mouse.up();
  await waitText("Archived 1 message");
  await clickText("Undo");
  await waitText("A little room for good ideas");
  await clickText("Preferences");
  await waitText("Swipe left");
  await capture("preferences-light");
  await openDropdown("Theme");
  await clickText("Dark");
  await waitText("Preferences saved");
  await page.screenshot({ path: path.join(out, "preferences-dark.png") });
  await clickText("Mail");
  await waitText("A little room for good ideas");
  await page.screenshot({ path: path.join(out, "inbox-dark.png") });
  await visualParity("dark");
  await page
    .getByRole("group", { name: /A little room for good ideas/ })
    .click();
  await clickText("Find in message");
  await enterFind("first");
  await waitText("1 of 1");
  await page.screenshot({ path: path.join(out, "find-dark.png") });
  await clickText("Match case");
  await enterFind("FIRST");
  await waitText("No matches");
  await clickText("Close Find");
  await clickText("Back");
  await clickText("Calendar");
  await waitText("September 2026");
  await page.screenshot({ path: path.join(out, "calendar-dark.png") });
  await clickText("Preferences");
  const googleCalendar = page.getByRole("button", { name: /Calendar access/ });
  await scrollToGoogle(googleCalendar);
  await openDropdown("Calendar access");
  await clickText("Read calendars");
  await scrollToGoogle(
    page.getByRole("button", { name: "Sign in with Google", exact: true }),
  );
  await clickText("Sign in with Google");
  await waitText("Google connection saved on this device.");
  await scrollToGoogle(googleCalendar);
  await openDropdown("Calendar access");
  await clickText("Read and edit calendars");
  await scrollToGoogle(
    page.getByRole("button", { name: "Reconnect Google", exact: true }),
  );
  await clickText("Reconnect Google");
  await waitText("Google sign-in was cancelled.");
  await waitText("Saved access: Drive off · Calendar read only");
  await page.mouse.move(210, 430);
  await page.mouse.wheel(0, 400);
  await page.waitForTimeout(80); // Frame settling for the visible error capture.
  await page.screenshot({ path: path.join(out, "google-cancelled-dark.png") });
  await clickText("Reconnect Google");
  await waitText("Calendar read and edit");
  await clickText("Disconnect…");
  await clickText("Cancel");
  await clickText("Disconnect…");
  await clickText("Disconnect");
  await waitText("Google disconnected on this device.");
  assert.deepEqual(errors, []);
  assert.deepEqual(
    externalRequests,
    [],
    "The fictional Flutter preview must not contact outside services",
  );
  await writeFile(
    path.join(out, "result.json"),
    JSON.stringify(
      {
        passed: true,
        scenarios: [
          "inbox",
          "swipe-archive",
          "undo",
          "appearance",
          "calendar",
          "Find-dark-case",
          "Google-consent-cancel-retry-disconnect",
        ],
        errors,
      },
      null,
      2,
    ),
  );
  console.log("PASS: Flutter browser inbox, real swipe, undo, theme, calendar");
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
