// Appium/UiAutomator2, like Walkie Textie's e2e/lib/phone.mjs. Only the
// isolated preview package on an explicitly selected emulator may be reset.
import { remote } from "webdriverio";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { setTimeout as delay } from "node:timers/promises";
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
const out = path.join(root, "artifacts/flutter/native");
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
async function waitText(text) {
  await driver.waitUntil(
    async () => {
      const src = await driver.getPageSource();
      return src.includes(text);
    },
    { timeout: 15000, timeoutMsg: `Missing visible native label: ${text}` },
  );
}
async function tap(text) {
  for (const selector of [
    `android=new UiSelector().description("${text}")`,
    `android=new UiSelector().text("${text}")`,
    `android=new UiSelector().descriptionStartsWith("${text}")`,
  ]) {
    const node = await driver.$(selector);
    if (await node.isExisting()) {
      await node.click();
      return;
    }
  }
  throw new Error(`Native control not found: ${text}`);
}
async function openDropdown(label) {
  const node = await driver.$(
    `android=new UiSelector().descriptionStartsWith("${label}")`,
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
async function step(name, fn) {
  await fn();
  steps.push(name);
  console.log(`PASS: ${name}`);
}
try {
  await step("isolated preview inbox", async () => {
    await waitText("A little room for good ideas");
    await waitText("FICTIONAL DATA");
    await driver.saveScreenshot(path.join(out, "inbox-light.png"));
  });
  await step("real native swipe and undo", async () => {
    const row = await driver.$(
      'android=new UiSelector().descriptionContains("A little room for good ideas")',
    );
    const pos = await row.getLocation();
    const size = await row.getSize();
    const { width } = await driver.getWindowSize();
    const y = pos.y + Math.min(size.height / 2, 100);
    // UiAutomator2 needs one complete pointer sequence. Capture through adb
    // during its intentional visual pause, without a second action driver.
    const gesture = driver.performActions([
      {
        type: "pointer",
        id: "finger",
        parameters: { pointerType: "touch" },
        actions: [
          {
            type: "pointerMove",
            duration: 0,
            x: Math.round(width * 0.88),
            y: Math.round(y),
          },
          { type: "pointerDown", button: 0 },
          {
            type: "pointerMove",
            duration: 500,
            x: Math.round(width * 0.12),
            y: Math.round(y),
          },
          { type: "pause", duration: 1600 },
          { type: "pointerUp", button: 0 },
        ],
      },
    ]);
    const capture = async () => {
      await delay(1000); // Photograph the held gesture, before releasing it.
      const { stdout } = await promisify(execFile)(
        "adb",
        ["-s", serial, "exec-out", "screencap", "-p"],
        { encoding: "buffer", maxBuffer: 8 * 1024 * 1024 },
      );
      await writeFile(path.join(out, "swipe-archive-icon.png"), stdout);
    };
    await Promise.all([gesture, capture()]);
    await driver.releaseActions();
    await waitText("Moved to Archive");
    await tap("Undo");
    await waitText("A little room for good ideas");
  });
  await step("native preferences and theme", async () => {
    await tap("Preferences");
    await waitText("Swipe left");
    await openDropdown("Theme");
    await tap("Dark");
    await waitText("Preferences saved");
    await driver.saveScreenshot(path.join(out, "preferences-dark.png"));
  });
  await step("native calendar navigation", async () => {
    await tap("Calendar");
    await waitText("September 2026");
    await driver.saveScreenshot(path.join(out, "calendar-dark.png"));
  });
  await step("native mail navigation after preferences", async () => {
    await tap("Mail");
    await waitText("A little room for good ideas");
    await driver.saveScreenshot(path.join(out, "inbox-dark.png"));
  });
  await step("native dark reader Find", async () => {
    await (
      await driver.$(
        'android=new UiSelector().descriptionContains("A little room for good ideas")',
      )
    ).click();
    await tap("Find in message");
    const input = await driver.$(
      'android=new UiSelector().className("android.widget.EditText")',
    );
    await input.setValue("first");
    await waitText("1 of 1");
    await driver.saveScreenshot(path.join(out, "find-dark.png"));
    await tap("Match case");
    await input.setValue("FIRST");
    await waitText("No matches");
    await tap("Close Find");
  });
  assert.equal(steps.length, 6);
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
