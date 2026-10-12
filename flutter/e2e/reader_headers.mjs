import { chromium } from "playwright";
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const out = path.join(root, "artifacts/flutter/reader-headers");
await mkdir(out, { recursive: true });
const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({ viewport: { width: 360, height: 800 }, permissions: ["clipboard-read", "clipboard-write"] });
const page = await context.newPage();
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
const button = (name) => page.getByRole("button", { name, exact: true })
  .or(page.getByRole("menuitem", { name, exact: true })).last();
async function waitText(text) {
  await page.waitForFunction((text) => [...document.querySelectorAll("flt-semantics,[aria-label],[aria-live],input,textarea")]
    .some((element) => ((element.textContent ?? "") + " " + (element.getAttribute("aria-label") ?? "") + " " + (element.value ?? "")).includes(text)), text);
}
async function reveal(name) {
  for (let attempt = 0; attempt < 16; attempt++) {
    const box = await button(name).boundingBox();
    if (box && box.y > 65 && box.y + box.height < 660) return;
    await page.mouse.move(330, 350);
    await page.mouse.wheel(0, box && box.y < 65 ? -260 : 260);
    await page.waitForTimeout(100);
  }
  throw new Error(`Could not reveal ${name}`);
}
async function copy(name, expected) {
  await reveal(name);
  await button(name).click();
  await waitText(`${name.substring(5)} copied.`);
  assert.equal(await page.evaluate(() => navigator.clipboard.readText()), expected);
}
async function refreshRecipient(expected) {
  await button("Refresh mail").click();
  await page.waitForFunction(() => ![...document.querySelectorAll("flt-semantics,[aria-label],[aria-live]")]
    .some((element) => ((element.textContent ?? "") + " " + (element.getAttribute("aria-label") ?? "")).includes("To copied.")));
  await copy("Copy To", expected);
}
try {
  await page.goto(process.env.SHEP_FLUTTER_URL);
  await page.waitForSelector("flt-semantics-placeholder", { state: "attached" });
  await page.evaluate(() => document.querySelector("flt-semantics-placeholder").click());
  await waitText("Café plans - ガ and 한글");
  await page.getByRole("group", { name: /Café plans/ }).click();
  await waitText("Account: Receiving account");
  await copy("Copy subject", "Café plans - ガ and 한글");
  await copy("Copy sender", String.raw`"Robin \"RJ\" Field" <sender@example.test>`);
  await copy("Copy sender address", "sender@example.test");
  await copy("Copy To", 'Alias <alias@example.test>, "Café Team" <team@example.test>');
  assert.equal(await page.locator('iframe[title="Formatted message"]').count(), 0);
  await page.screenshot({ path: path.join(out, "headers-before-body.png") });
  await refreshRecipient("Refreshed 1 <refresh-1@example.test>");
  const frame = page.frameLocator('iframe[title="Formatted message"]');
  await frame.getByRole("heading", { name: "Verification needed" }).waitFor();
  await page.evaluate(() => { window.savedHeaderFrame = document.querySelector('iframe[title="Formatted message"]').contentWindow; });
  await refreshRecipient("Refreshed 2 <refresh-2@example.test>");
  assert.equal(await page.evaluate(() => window.savedHeaderFrame === document.querySelector('iframe[title="Formatted message"]').contentWindow), true);
  await page.screenshot({ path: path.join(out, "headers-refreshed-frame-retained.png") });
  await button("Back").click();
  await page.getByRole("tab", { name: "Preferences", exact: true }).click();
  await waitText("Theme");
  const theme = await page.getByRole("button", { name: /^Theme/ }).boundingBox();
  await page.mouse.click(theme.x + theme.width - 36, theme.y + theme.height / 2);
  await button("Dark").click();
  await waitText("Preferences saved");
  await page.getByRole("tab", { name: "Mail", exact: true }).click();
  await page.getByRole("group", { name: /Café plans/ }).click();
  await waitText("Account: Receiving account");
  await copy("Copy subject", "Café plans - ガ and 한글");
  await copy("Copy To", "Refreshed 2 <refresh-2@example.test>");
  await frame.getByRole("heading", { name: "Verification needed" }).waitFor();
  await page.screenshot({ path: path.join(out, "headers-dark.png") });
  assert.deepEqual(errors, []);
  await writeFile(path.join(out, "result.json"), JSON.stringify({ passed: true, scenarios: ["header-copy-before-body", "metadata-refresh-retains-frame"], errors }, null, 2));
  console.log("PASS: actual reader header clipboard and retained formatted frame");
} catch (error) {
  await page.screenshot({ path: path.join(out, "failure.png") });
  await writeFile(path.join(out, "failure.html"), await page.content());
  throw error;
} finally {
  await browser.close();
}
