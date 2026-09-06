import { chromium } from "playwright";
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 412, height: 892 } });
await page.goto("http://127.0.0.1:5181");
await page.waitForSelector("flt-semantics-placeholder", { state: "attached" });
await page.evaluate(() =>
  document.querySelector("flt-semantics-placeholder").click(),
);
await page.waitForFunction(
  () => document.querySelectorAll("flt-semantics").length > 10,
);
if (process.argv.includes("--preferences")) {
  await page.getByRole("tab", { name: "Preferences" }).click();
  await page.waitForTimeout(300);
}
if (process.argv.includes("--theme")) {
  const b = await page.getByRole("button", { name: /^Theme/ }).boundingBox();
  await page.mouse.click(b.x + b.width - 36, b.y + b.height / 2);
  await page.waitForTimeout(300);
}
console.log(
  JSON.stringify(
    await page.locator("flt-semantics").evaluateAll((nodes) =>
      nodes.map((e) => ({
        text: e.textContent,
        aria: e.getAttribute("aria-label"),
        role: e.getAttribute("role"),
      })),
    ),
    null,
    2,
  ),
);
await browser.close();
