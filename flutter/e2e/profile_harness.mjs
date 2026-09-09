// One saved control flow on Flutter web semantics and Android UiAutomator2.
// Provider outcomes are isolated test fixtures; all actions use actual controls.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { setTimeout as delay } from "node:timers/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
export async function withProfileHarness(mode, name, run) {
  assert.ok(["web", "native"].includes(mode));
  const root = path.resolve(
    path.dirname(fileURLToPath(import.meta.url)),
    "../..",
  );
  const out = path.join(root, `artifacts/flutter/${name}-${mode}`);
  await mkdir(out, { recursive: true });
  const steps = [],
    errors = [],
    external = [];
  let page, browser, driver;
  const textOf = async () =>
    mode === "web" ? page.locator("body").innerText() : driver.getPageSource();
  async function labels() {
    if (mode === "native") return textOf();
    return page
      .locator("flt-semantics,[aria-label]")
      .evaluateAll((nodes) =>
        nodes
          .map(
            (n) =>
              `${n.textContent ?? ""} ${n.getAttribute("aria-label") ?? ""}`,
          )
          .join("\n"),
      );
  }
  async function node(label, button = false) {
    if (mode === "web") {
      // Flutter web joins a list tile's title and subtitle in one clickable
      // node, so a subtitle label may start after whitespace-normalised text.
      const name = new RegExp(
        `(?:^|\\s)${label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(?:$|\\s)`,
      );
      return button
        ? page
            .getByRole("button", { name })
            .or(page.getByRole("tab", { name }))
            .or(page.getByRole("menuitem", { name }))
            .last()
        : page
            .getByText(label, { exact: false })
            .or(page.getByLabel(label, { exact: false }))
            .last();
    }
    for (const attribute of ["description", "text"]) {
      const selector = `android=new UiSelector().${attribute}Contains(${JSON.stringify(label)})${button ? ".clickable(true)" : ""}`;
      const nodes = await driver.$$(selector);
      if (nodes.length) return nodes[nodes.length - 1];
    }
    return null;
  }
  async function box(label, button = false) {
    const target = await node(label, button);
    if (!target) return null;
    if (mode === "web")
      return (await target.count()) ? target.boundingBox() : null;
    if (!(await target.isExisting()) || !(await target.isDisplayed()))
      return null;
    return { ...(await target.getLocation()), ...(await target.getSize()) };
  }
  async function scroll(direction) {
    if (mode === "web") {
      await page.mouse.move(300, 480);
      await page.mouse.wheel(0, direction * 330);
      await page.waitForTimeout(100);
      return;
    }
    const { width, height } = await driver.getWindowSize();
    await driver.performActions([
      {
        type: "pointer",
        id: "profile-scroll",
        parameters: { pointerType: "touch" },
        actions: [
          {
            type: "pointerMove",
            duration: 0,
            x: Math.round(width * 0.6),
            y: Math.round(height * (direction > 0 ? 0.75 : 0.3)),
          },
          { type: "pointerDown", button: 0 },
          {
            type: "pointerMove",
            duration: 400,
            x: Math.round(width * 0.6),
            y: Math.round(height * (direction > 0 ? 0.3 : 0.75)),
          },
          { type: "pointerUp", button: 0 },
        ],
      },
    ]);
    await driver.releaseActions();
  }
  async function reveal(label, { button = false, direction = 1 } = {}) {
    const height = mode === "web" ? 892 : (await driver.getWindowSize()).height;
    for (let i = 0; i < 16; i++) {
      const bounds = await box(label, button);
      if (bounds && bounds.y > 70 && bounds.y + bounds.height < height - 35)
        return;
      await scroll(bounds && bounds.y < 70 ? -1 : direction);
    }
    throw new Error(`Could not reveal profile control: ${label}`);
  }
  async function tap(label) {
    // A completed Back tap can precede the next route's accessibility tree.
    // Wait for the actual clickable control under the existing result deadline.
    let target;
    if (mode === "native") {
      await driver.waitUntil(
        async () => {
          target = await node(label, true);
          return (
            target &&
            (await target.isExisting()) &&
            (await target.isDisplayed())
          );
        },
        { timeout: 15000, timeoutMsg: `Missing actual control: ${label}` },
      );
    } else target = await node(label, true);
    await target.click();
  }
  async function wait(label) {
    if (mode === "native")
      return driver.waitUntil(async () => (await labels()).includes(label), {
        timeout: 15000,
        timeoutMsg: `Missing profile result: ${label}`,
      });
    await page.waitForFunction(
      (label) =>
        [...document.querySelectorAll("flt-semantics,[aria-label]")].some((n) =>
          `${n.textContent ?? ""} ${n.getAttribute("aria-label") ?? ""}`.includes(
            label,
          ),
        ),
      label,
    );
  }
  async function capture(name) {
    // State can settle before the native tap ripple finishes painting.
    await delay(200);
    if (mode === "web")
      await page.screenshot({ path: path.join(out, `${name}.png`) });
    else await driver.saveScreenshot(path.join(out, `${name}.png`));
    await writeFile(path.join(out, `${name}.txt`), await labels());
  }
  async function theme(value) {
    await reveal("Theme", { direction: -1 });
    let bounds = await box("Theme", mode === "web");
    assert.ok(bounds, "Theme dropdown must have a painted target");
    if (mode === "web")
      await page.mouse.click(
        bounds.x + bounds.width - 36,
        bounds.y + bounds.height / 2,
      );
    else {
      await driver.performActions([
        {
          type: "pointer",
          id: "profile-theme",
          parameters: { pointerType: "touch" },
          actions: [
            {
              type: "pointerMove",
              duration: 0,
              x: Math.round(bounds.x + bounds.width * 0.8),
              y: Math.round(bounds.y + bounds.height / 2),
            },
            { type: "pointerDown", button: 0 },
            { type: "pointerUp", button: 0 },
          ],
        },
      ]);
      await driver.releaseActions();
    }
    await wait(value);
    await tap(value);
  }
  try {
    if (mode === "web") {
      const { chromium } = await import("playwright");
      browser = await chromium.launch({ headless: true });
      page = await browser.newPage({ viewport: { width: 412, height: 892 } });
      page.on("pageerror", (e) => errors.push(e.message));
      await page.route("**/*", async (route) => {
        const url = new URL(route.request().url());
        if (
          ["http:", "https:"].includes(url.protocol) &&
          !["localhost", "127.0.0.1"].includes(url.hostname)
        ) {
          external.push(url.origin + url.pathname);
          return route.abort();
        }
        return route.continue();
      });
      await page.goto(process.env.SHEP_FLUTTER_URL ?? "http://127.0.0.1:5181");
      await page.waitForSelector("flt-semantics-placeholder", {
        state: "attached",
      });
      await page.evaluate(() =>
        document.querySelector("flt-semantics-placeholder").click(),
      );
    } else {
      const serial = process.env.ANDROID_SERIAL;
      assert.match(
        serial ?? "",
        /^emulator-\d+$/,
        "Only an explicit emulator may run this fixture",
      );
      const { stdout } = await promisify(execFile)("adb", [
        "-s",
        serial,
        "emu",
        "avd",
        "name",
      ]);
      assert.ok(
        stdout.split("\n")[0].startsWith("shep-e2e"),
        "Use the isolated Shep AVD",
      );
      const { remote } = await import("webdriverio");
      driver = await remote({
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
    }
    await run({
      mode,
      page,
      driver,
      steps,
      labels,
      node,
      box,
      scroll,
      reveal,
      tap,
      wait,
      capture,
      theme,
    });
    assert.deepEqual(errors, []);
    assert.deepEqual(external, []);
    await writeFile(
      path.join(out, "result.json"),
      JSON.stringify(
        { steps, errors, external, provider: "isolated fixture" },
        null,
        2,
      ) + "\n",
    );
    console.log(
      `Flutter ${mode} profile controls passed: ${steps.length} flows.`,
    );
  } catch (error) {
    if (page || driver) await capture("failure").catch(() => {});
    throw error;
  } finally {
    if (driver) await driver.deleteSession();
    if (browser) await browser.close();
  }
}
