// Appium/UiAutomator2 group action controls on the isolated preview package:
// Select mode, a real long press, captured Select all, the frozen review,
// progress with Pause/Resume, Undo and History, in light and dark.
import { remote } from "webdriverio";
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
const serial = process.env.ANDROID_SERIAL;
if (!serial?.startsWith("emulator-"))
  throw new Error(
    "Set ANDROID_SERIAL to an isolated emulator ID. Personal devices are refused.",
  );
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const out = path.join(root, "artifacts/flutter/native/bulk");
await mkdir(out, { recursive: true });
const driver = await remote({
  hostname: "127.0.0.1",
  port: Number(process.env.APPIUM_PORT ?? 4725),
  path: "/",
  logLevel: "error",
  capabilities: {
    platformName: "Android",
    "appium:automationName": "UiAutomator2",
    "appium:udid": serial,
    "appium:appPackage": "so.shep.shep_mobile.preview",
    "appium:appActivity": "so.shep.shep_mobile.MainActivity",
    "appium:noReset": false,
    "appium:autoGrantPermissions": true,
    "appium:newCommandTimeout": 180,
  },
});
const steps = [];
// Selector lookups are much cheaper than full hierarchy dumps, which matters
// while a group runs and its notices change every few hundred milliseconds.
async function hasText(text) {
  const escaped = text.replace(/"/g, '\\"');
  for (const selector of [
    `android=new UiSelector().descriptionContains("${escaped}")`,
    `android=new UiSelector().textContains("${escaped}")`,
  ]) {
    if (await (await driver.$(selector)).isExisting()) return true;
  }
  return false;
}
async function waitText(text, timeout = 20000) {
  await driver.waitUntil(async () => hasText(text), {
    timeout,
    interval: 250,
    timeoutMsg: `Missing visible native label: ${text}`,
  });
}
async function tap(text) {
  for (const selector of [
    `android=new UiSelector().description("${text}").clickable(true)`,
    `android=new UiSelector().text("${text}").clickable(true)`,
    `android=new UiSelector().descriptionStartsWith("${text}").clickable(true)`,
    `android=new UiSelector().description("${text}")`,
    `android=new UiSelector().text("${text}")`,
  ]) {
    const node = await driver.$(selector);
    if (await node.isExisting()) {
      await node.click();
      return;
    }
  }
  throw new Error(`Native control not found: ${text}`);
}
// Android merges a History card header (title, status and expand label) into
// one clickable node, so the expand label is matched inside that node.
async function tapMerged(label) {
  const node = await driver.$(
    `android=new UiSelector().descriptionContains("${label}").clickable(true)`,
  );
  if (!(await node.isExisting()))
    throw new Error(`Native control not found: ${label}`);
  await node.click();
}
async function openDropdown(label) {
  const node = await driver.$(
    `android=new UiSelector().descriptionContains("${label}")`,
  );
  const pos = await node.getLocation();
  const size = await node.getSize();
  await driver.performActions([
    {
      type: "pointer",
      id: "choice",
      parameters: { pointerType: "touch" },
      actions: [
        {
          type: "pointerMove",
          duration: 0,
          x: Math.round(pos.x + size.width * 0.88),
          y: Math.round(pos.y + size.height / 2),
        },
        { type: "pointerDown", button: 0 },
        { type: "pointerUp", button: 0 },
      ],
    },
  ]);
  await driver.releaseActions();
}
async function longPressRow(description) {
  const row = await driver.$(
    `android=new UiSelector().descriptionContains("${description}")`,
  );
  const pos = await row.getLocation();
  const size = await row.getSize();
  await driver.performActions([
    {
      type: "pointer",
      id: "finger",
      parameters: { pointerType: "touch" },
      actions: [
        {
          type: "pointerMove",
          duration: 0,
          x: Math.round(pos.x + size.width * 0.45),
          y: Math.round(pos.y + Math.min(size.height / 2, 90)),
        },
        { type: "pointerDown", button: 0 },
        { type: "pause", duration: 900 },
        { type: "pointerUp", button: 0 },
      ],
    },
  ]);
  await driver.releaseActions();
}
// A touch that lands while the list is still settling can be dropped; the
// control is retried until its visible effect appears.
async function tapUntil(text, expected) {
  for (let attempt = 0; ; attempt++) {
    await tap(text);
    try {
      await waitText(expected, 6000);
      return;
    } catch (error) {
      if (attempt === 2) throw error;
    }
  }
}
async function step(name, fn) {
  await fn();
  steps.push(name);
  console.log(`PASS: ${name}`);
}
async function selectAll() {
  await tapUntil("Select", "No messages selected");
  await tapUntil("Select all", "All 130 selected");
}
try {
  await step("native select mode, checkbox, long-press range and Select all", async () => {
    await waitText("A little room for good ideas");
    await waitText("PREVIEW");
    await tapUntil("Select", "No messages selected");
    await tapUntil("Select A little room for good ideas", "1 selected");
    await longPressRow("Coffee on Thursday?, not selected");
    await waitText("3 selected");
    await tap("Select all");
    await waitText("All 130 selected");
    await driver.saveScreenshot(path.join(out, "bulk-selection-light.png"));
    await tap("Clear");
    await waitText("No messages selected");
  });
  await step("native frozen review counts and decline", async () => {
    await tap("Select all");
    await waitText("All 130 selected");
    await tap("Archive selected");
    await waitText("Archive 130 messages");
    await waitText("Personal · Inbox: 86");
    await waitText("Work · Inbox: 44");
    await driver.saveScreenshot(path.join(out, "bulk-review-light.png"));
    await tap("Cancel");
    await driver.waitUntil(async () => !(await hasText("Archive 130 messages")), {
      timeout: 15000,
      timeoutMsg: "Declined review stayed open",
    });
  });
  await step("native approve, pause, resume, completion and Undo", async () => {
    await selectAll();
    await tap("Archive selected");
    await waitText("Archive 130 messages");
    await tap("Archive");
    await waitText("Archiving, ");
    await waitText("All clear", 10000);
    await driver.saveScreenshot(path.join(out, "bulk-progress-light.png"));
    await tap("Pause");
    await waitText("Paused");
    await driver.saveScreenshot(path.join(out, "bulk-paused-light.png"));
    await tap("Resume");
    await waitText("Archived 130", 180000);
    await driver.saveScreenshot(path.join(out, "bulk-complete-light.png"));
    await tap("Undo");
    await waitText("A little room for good ideas");
    await waitText("Undone: 130 restored", 180000);
    await tap("Dismiss group notification");
  });
  await step("native dark flag group and History items", async () => {
    await tap("Preferences");
    await waitText("Swipe left");
    await openDropdown("Theme");
    await tap("Dark");
    await waitText("Preferences saved");
    await tap("Mail");
    await waitText("A little room for good ideas");
    await selectAll();
    await driver.saveScreenshot(path.join(out, "bulk-selection-dark.png"));
    await tap("Mark read selected");
    await waitText("Mark read 130 messages");
    await driver.saveScreenshot(path.join(out, "bulk-review-dark.png"));
    await tap("Mark read");
    await waitText("Marked read 65, 65 skipped", 180000);
    await driver.saveScreenshot(path.join(out, "bulk-complete-dark.png"));
    await tap("Open navigation menu");
    await waitText("Group History");
    await tap("Group History");
    await waitText("Mark read 130 messages");
    await waitText("Archive 130 messages");
    await tapMerged("Show messages of Mark read 130 messages");
    await waitText("Skipped · Already up to date");
    await driver.saveScreenshot(path.join(out, "bulk-history-dark.png"));
    await driver.back();
    await waitText("A little room for good ideas");
  });
  assert.equal(steps.length, 4);
  await writeFile(
    path.join(out, "result.json"),
    JSON.stringify({ passed: true, serial, steps }, null, 2),
  );
} catch (error) {
  await driver.saveScreenshot(path.join(out, "failure.png"));
  await writeFile(path.join(out, "failure.xml"), await driver.getPageSource());
  throw error;
} finally {
  await driver.deleteSession();
}
