// Appium/UiAutomator2, like Walkie Textie's e2e/lib/phone.mjs. Only the
// isolated preview package on an explicitly selected emulator may be reset.
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
const out = path.join(root, "artifacts/flutter/native/formatted-appium");
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
await driver.updateSettings({ enableMultiWindows: true });
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
    `android=new UiSelector().descriptionContains("${text}")`,
  ]) {
    const node = await driver.$(selector);
    if (await node.isExisting()) {
      await node.click();
      return;
    }
  }
  const floating = await driver.$(
    `//*[@text=${JSON.stringify(text)} or @content-desc=${JSON.stringify(text)}]`,
  );
  if (await floating.isExisting()) {
    await floating.click();
    return;
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
async function reveal(text, direction = "up") {
  for (let attempt = 0; attempt < 16; attempt++) {
    const source = await driver.getPageSource();
    if (source.includes(text)) {
      const exactText = await driver.$(
        `android=new UiSelector().text("${text}")`,
      );
      const exactDescription = await driver.$(
        `android=new UiSelector().description("${text}")`,
      );
      const target = (await exactText.isExisting())
        ? exactText
        : (await exactDescription.isExisting())
          ? exactDescription
          : await driver.$(
              `android=new UiSelector().descriptionContains("${text}")`,
            );
      if (await target.isExisting()) {
        const p = await target.getLocation(),
          size = await target.getSize(),
          screen = await driver.getWindowSize();
        if (
          p.y > screen.height * 0.08 &&
          p.y + size.height < screen.height * 0.9
        )
          return;
      }
    }
    const screen = await driver.getWindowSize();
    await driver.execute("mobile: swipeGesture", {
      left: screen.width - 45,
      top: Math.round(screen.height * 0.3),
      width: 25,
      height: Math.round(screen.height * 0.45),
      direction,
      percent: 0.8,
    });
  }
  throw new Error(`Could not reveal ${text}`);
}
async function query(value, status) {
  const input = await driver.$(
    'android=new UiSelector().className("android.widget.EditText")',
  );
  await input.click();
  await input.clearValue();
  await input.setValue(value);
  await waitText(status);
  if (await driver.isKeyboardShown()) await driver.pressKeyCode(4);
}
try {
  await step("real native prepared HTML and Retry", async () => {
    await waitText("A little room for good ideas");
    await tap("A little room for good ideas");
    await waitText("Retry formatted message");
    await reveal("Retry formatted message");
    await tap("Retry formatted message");
    await waitText("Verification needed");
    await reveal("Verification needed");
    await driver.saveScreenshot(path.join(out, "formatted-native-light.png"));
  });
  await step("Android selection menu Copy and actual paste", async () => {
    const heading = await driver.$(
      'android=new UiSelector().text("Verification needed")',
    );
    const p = await heading.getLocation(),
      size = await heading.getSize();
    await driver.performActions([
      {
        type: "pointer",
        id: "selection",
        parameters: { pointerType: "touch" },
        actions: [
          {
            type: "pointerMove",
            duration: 0,
            x: Math.round(p.x + size.width * 0.25),
            y: Math.round(p.y + size.height / 2),
          },
          { type: "pointerDown", button: 0 },
          { type: "pause", duration: 1000 },
          { type: "pointerUp", button: 0 },
        ],
      },
    ]);
    await driver.releaseActions();
    await waitText("Copy");
    await driver.saveScreenshot(
      path.join(out, "formatted-native-selection.png"),
    );
    await tap("Copy");
    await tap("Find in message");
    const input = await driver.$(
      'android=new UiSelector().className("android.widget.EditText")',
    );
    await input.click();
    const inputPos = await input.getLocation(),
      inputSize = await input.getSize();
    await driver.execute("mobile: longClickGesture", {
      x: Math.round(inputPos.x + inputSize.width * 0.25),
      y: Math.round(inputPos.y + inputSize.height * 0.25),
      duration: 1000,
    });
    await waitText("Paste");
    await tap("Paste");
    await waitText("1 of 1");
    assert.ok((await input.getText()).includes("Verification"));
    if (await driver.isKeyboardShown()) await driver.pressKeyCode(4);
  });
  await step("native formatted Find, quotes and plain choice", async () => {
    await query("Alpha across spans", "1 of 1");
    await query("Alpha", "1 of 2");
    await reveal("Show quoted history", "down");
    await tap("Show quoted history");
    await waitText("1 of 3");
    await tap("Next match");
    await waitText("2 of 3");
    await tap("Next match");
    await waitText("3 of 3");
    await driver.saveScreenshot(path.join(out, "formatted-native-tail.png"));
    await reveal("Plain text", "down");
    await tap("Plain text");
    await waitText("1 of 2");
    await reveal("Formatted", "down");
    await tap("Formatted");
    await waitText("1 of 3");
  });
  await step("actual native HTML link and address Copy", async () => {
    await query("Read the help page", "1 of 1");
    await waitText("Read the help page");
    await tap("Read the help page");
    await waitText("Message link");
    await tap("Copy address");
    await waitText("Address copied.");
    await tap("Close");
  });
  await step("native dark formatted reader", async () => {
    await tap("Close Find");
    await tap("Back");
    await tap("Preferences");
    await waitText("Theme");
    await openDropdown("Theme");
    await tap("Dark");
    await waitText("Preferences saved");
    await tap("Mail");
    await tap("A little room for good ideas");
    await waitText("Verification needed");
    await tap("Find in message");
    await query("Café", "1 of 3");
    await driver.saveScreenshot(path.join(out, "formatted-native-dark.png"));
  });
  await step(
    "native footer Move and Reply after formatted scrolling",
    async () => {
      await tap("Close Find");
      const screen = await driver.getWindowSize();
      for (const name of ["Reply", "Reply all", "Forward", "Print", "Move"]) {
        const node = await driver.$(
          `android=new UiSelector().description("${name}")`,
        );
        assert.ok(await node.isDisplayed(), name);
        const pos = await node.getLocation(),
          size = await node.getSize();
        assert.ok(
          pos.y > screen.height / 2 && pos.y + size.height <= screen.height,
          `${name}: ${JSON.stringify({ pos, size })}`,
        );
      }
      await driver.saveScreenshot(path.join(out, "reader-footer-dark.png"));
      await tap("Move");
      await waitText("Move message");
      await driver.back();
      await tap("Reply");
      await waitText("Re: A little room for good ideas");
    },
  );
  await writeFile(
    path.join(out, "result.json"),
    JSON.stringify({ passed: true, steps }, null, 2),
  );
} catch (error) {
  await driver.saveScreenshot(path.join(out, "failure.png"));
  await writeFile(path.join(out, "failure.xml"), await driver.getPageSource());
  throw error;
} finally {
  await driver.deleteSession();
}
