// Actual Flutter controls + iframe text input, sharing current Rust-prepared MIME.
import { chromium } from "playwright";
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const out = path.join(root, "artifacts/flutter/web-formatted");
await mkdir(out, { recursive: true });
const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({
  viewport: { width: 412, height: 892 },
  permissions: ["clipboard-read", "clipboard-write"],
});
const page = await context.newPage(),
  errors = [],
  requests = [];
page.on("pageerror", (e) => errors.push(e.message));
page.on("request", (r) => {
  if (new URL(r.url()).hostname.endsWith("example.test"))
    requests.push(r.url());
});
const frame = page.frameLocator('iframe[title="Formatted message"]');
async function waitText(text) {
  await page.waitForFunction(
    (t) =>
      [
        ...document.querySelectorAll(
          "flt-semantics,[aria-label],[aria-live],input,textarea",
        ),
      ].some((e) =>
        (
          (e.textContent ?? "") +
          " " +
          (e.getAttribute("aria-label") ?? "") +
          " " +
          (e.value ?? "")
        ).includes(t),
      ),
    text,
  );
}
const button = (name) =>
  page
    .getByRole("button", { name, exact: true })
    .or(page.getByRole("tab", { name, exact: true }))
    .or(page.getByRole("menuitem", { name, exact: true }))
    .last();
async function click(name) {
  await button(name).click();
}
async function reveal(name) {
  for (let i = 0; i < 16; i++) {
    const box = await button(name).boundingBox();
    if (box && box.y > 70 && box.y + box.height < 800) return;
    await page.mouse.move(405, 500);
    await page.mouse.wheel(0, box && box.y < 70 ? -360 : 360);
    await page.waitForTimeout(100);
  }
  throw new Error(`Could not reveal ${name}`);
}
async function query(value, status) {
  const close = await button("Close Find").boundingBox();
  assert.ok(close);
  await page.mouse.click(close.x / 2, close.y + close.height / 2);
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.type(value);
  await waitText(status);
}
try {
  await page.goto(process.env.SHEP_FLUTTER_URL);
  await page.waitForSelector("flt-semantics-placeholder", {
    state: "attached",
  });
  await page.evaluate(() =>
    document.querySelector("flt-semantics-placeholder").click(),
  );
  await waitText("A little room for good ideas");
  await page
    .getByRole("group", { name: /A little room for good ideas/ })
    .click();
  await waitText("Retry formatted message");
  await reveal("Retry formatted message");
  await click("Retry formatted message");
  await frame.getByRole("heading", { name: "Verification needed" }).waitFor();
  await reveal("Formatted");
  await page.screenshot({ path: path.join(out, "formatted-light.png") });
  await click("Find in message");
  await query("Alpha across spans", "1 of 1");
  assert.equal(
    await frame
      .locator("body")
      .evaluate(() => CSS.highlights.get("shep-active")?.size),
    3,
  );
  await query("Alpha", "1 of 2");
  await reveal("Show quoted history");
  await click("Show quoted history");
  await waitText("1 of 3");
  await click("Next match");
  await waitText("2 of 3");
  await click("Next match");
  await waitText("3 of 3");
  assert.ok((await frame.locator("body").evaluate(() => scrollY)) > 1000);
  await page.screenshot({ path: path.join(out, "formatted-tail.png") });
  await reveal("Plain text");
  await click("Plain text");
  await waitText("1 of 2");
  await query("Plain alternative", "1 of 1");
  await query("Alpha", "1 of 2");
  await reveal("Formatted");
  await click("Formatted");
  await waitText("1 of 3");
  await query("Read the help page", "1 of 1");
  const link = frame.getByRole("link", {
    name: "Read the help page",
    exact: true,
  });
  const linkDeadline = Date.now() + 10000;
  let linkBox;
  while (Date.now() < linkDeadline) {
    linkBox = await link.boundingBox();
    if (linkBox && linkBox.y >= 150 && linkBox.y + linkBox.height < 780) break;
    await page.waitForTimeout(50);
  }
  assert.ok(
    linkBox && linkBox.y >= 150 && linkBox.y + linkBox.height < 780,
    JSON.stringify(linkBox),
  );
  await page.mouse.click(
    linkBox.x + Math.min(linkBox.width / 2, 20),
    linkBox.y + linkBox.height / 2,
  );
  await waitText("Message link");
  await click("Copy address");
  await waitText("Address copied.");
  await click("Close");
  await query("Alpha across spans", "1 of 1");
  assert.equal(
    await frame.locator("body").evaluate(() => {
      try {
        return parent.document.title;
      } catch {
        return "isolated";
      }
    }),
    "isolated",
  );
  await click("Close Find");
  await click("Back");
  await click("Preferences");
  await waitText("Theme");
  const theme = await page
    .getByRole("button", { name: /^Theme/ })
    .boundingBox();
  await page.mouse.click(
    theme.x + theme.width - 36,
    theme.y + theme.height / 2,
  );
  await click("Dark");
  await waitText("Preferences saved");
  await click("Mail");
  await page
    .getByRole("group", { name: /A little room for good ideas/ })
    .click();
  await frame.getByRole("heading", { name: "Verification needed" }).waitFor();
  await click("Find in message");
  await query("Café", "1 of 3");
  await page.screenshot({ path: path.join(out, "formatted-dark.png") });
  assert.deepEqual(errors, []);
  assert.deepEqual(requests, []);
  await writeFile(
    path.join(out, "result.json"),
    JSON.stringify(
      {
        passed: true,
        scenarios: [
          "preparation-retry",
          "authored-HTML",
          "inline-Find",
          "quote-scope",
          "next-and-scroll",
          "plain-choice",
          "link-copy",
          "opaque-frame-no-network",
          "dark-reader",
        ],
        errors,
      },
      null,
      2,
    ),
  );
  console.log("PASS: Flutter formatted reader controls");
} catch (error) {
  console.error("Browser errors:", errors);
  await page.screenshot({ path: path.join(out, "failure.png") });
  await writeFile(path.join(out, "failure.html"), await page.content());
  throw error;
} finally {
  await browser.close();
}
