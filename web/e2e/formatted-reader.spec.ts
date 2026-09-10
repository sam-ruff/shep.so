import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
const raw = readFileSync(
  new URL("../../shared/html-reader-fixture.eml", import.meta.url),
).toString("base64");
const runtime = readFileSync(
  new URL("../../shared/mail-content/src/document/runtime.js", import.meta.url),
);
const runtimeHash = createHash("sha256").update(runtime).digest("base64");
const profile = "H".repeat(43);
const subject = "Shep formatted reader fixture";
const attack = `Content-Type: text/html; charset=utf-8\r\n\r\n
<style>body{user-select:none!important;background:#fff;color:#111}.tracker{background:u\\72l(https://images.example.test/css)}@import 'https://images.example.test/stylesheet';.inject:before{content:"\\3c /style>\\3c script>ATTACK()\\3c /script>"}</style>
<script>parent.document.body.textContent='ATTACK';localStorage.setItem('ATTACK','yes');fetch('https://images.example.test/script')</script>
<iframe src="https://images.example.test/frame"></iframe><meta http-equiv=refresh content="0;url=https://images.example.test/nav"><form action="https://images.example.test/form"><input autofocus></form>
<p class=tracker>Selectable hostile message.</p><img onerror="ATTACK()" src="https://images.example.test/image"><a href="https://example.test/help" target=_top ping="https://images.example.test/ping">Safe link</a><a href="javascript:ATTACK()">Unsafe link</a>`;
async function seed(page: Page) {
  await page.route("**/api/session", (route) =>
    route.fulfill({
      json: {
        email: "owner@example.test",
        csrf: "c".repeat(43),
        user_id: profile,
      },
    }),
  );
  await page.route("**/seed-reader", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: "<!doctype html><title>Isolated cache setup</title>",
    }),
  );
  await page.goto("/seed-reader");
  // Fixture setup precedes mounting the production application. All subsequent
  // interaction uses actual controls; the cache is not an action API.
  await page.evaluate(
    async ({ profile, raw, attack, subject }) => {
      const { BrowserStore } = await import("/src/storage.ts");
      const store = await BrowserStore.open(profile);
      const account = {
        id: "fixture",
        name: "Reader fixture",
        email: "owner@example.test",
        protocol: "Pop3",
        host: "mail.example.test",
        port: 995,
        username: "fixture",
        incoming_security: "Tls",
        incoming_auth: "Password",
        smtp_host: "mail.example.test",
        smtp_port: 465,
        smtp_username: "fixture",
        smtp_security: "Tls",
        smtp_auth: "Automatic",
        smtp_separate_password: false,
        sent_copy: "LocalOnly",
        sent_folder: "Sent",
      };
      const changes: any[] = [
        { store: "accounts", key: account.id, value: account },
      ];
      for (const [id, title, mime, text] of [
        [
          "formatted",
          subject,
          raw,
          "Plain alternative: verification needed.\n\nAlpha in plain text.\n> Alpha in plain quoted history.",
        ],
        [
          "plain",
          "Plain message fixture",
          btoa("Content-Type: text/plain\r\n\r\nAnother plain message."),
          "Another plain message.",
        ],
        [
          "hostile",
          "Hostile message fixture",
          btoa(attack),
          "Selectable hostile message.",
        ],
      ]) {
        const core = {
          id,
          account_id: account.id,
          remote_id: id,
          folder: "INBOX",
          sender: "Shep preview <preview@example.test>",
          recipient: account.email,
          subject: title,
          preview: text,
          timestamp: 1788692400,
          unread: false,
          starred: false,
          attachment_count: 0,
        };
        changes.push(
          { store: "mail", key: id, value: { core, text } },
          { store: "raw", key: id, value: mime },
        );
      }
      await store.commit(changes);
      store.close();
    },
    { profile, raw, attack, subject },
  );
  // Match the production policy for srcdoc inheritance, including its exact
  // display-runtime hash. The Rust HTTPS suite separately verifies that header.
  await page.route(/\/$/, async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      headers: {
        ...response.headers(),
        "content-security-policy": `default-src 'self'; script-src 'self' 'wasm-unsafe-eval' 'sha256-${runtimeHash}'; worker-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; frame-src 'self'; frame-ancestors 'none'; form-action 'self'; base-uri 'none'`,
      },
    });
  });
  await page.goto("/");
}
const frame = (page: Page) =>
  page.frameLocator('iframe[title="Formatted message"]');
async function open(page: Page, title = subject) {
  await page.getByRole("button", { name: title, exact: true }).dblclick();
}
test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 920 });
  await seed(page);
});

test("formatted layout, inline image, real Copy, cross-span Find and quoted history", async ({
  page,
  context,
}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await open(page);
  const heading = frame(page).getByRole("heading", {
    name: "Verification needed",
  });
  await expect(heading).toBeVisible();
  await expect(frame(page).locator("table.card")).toHaveCSS(
    "background-color",
    "rgb(27, 27, 32)",
  );
  await expect
    .poll(() =>
      frame(page)
        .getByAltText("Shep dog")
        .evaluate((image: HTMLImageElement) => image.naturalWidth),
    )
    .toBeGreaterThan(0);
  await expect(
    page.getByText("1 remote image blocked.", { exact: true }),
  ).toBeVisible();
  await expect(frame(page).locator("blockquote")).toHaveCount(0);
  await heading.click();
  await page.keyboard.press("Control+a");
  await expect
    .poll(() =>
      frame(page)
        .locator("body")
        .evaluate(() => window.getSelection()?.toString()),
    )
    .toContain("Verification needed");
  await expect(
    page.getByRole("button", { name: "Done", exact: true }),
  ).toHaveCount(0);
  await heading.click({ clickCount: 3 });
  await page.keyboard.press("Control+c");
  await page.keyboard.press("Control+f");
  const input = page.getByRole("textbox", {
    name: "Find in message",
    exact: true,
  });
  await expect(input).toBeFocused();
  await page.keyboard.press("Control+v");
  await expect(input).toHaveValue(/Verification needed/);
  await input.fill("Alpha across spans");
  await expect(page.locator(".find-status")).toHaveText("1 of 1");
  await expect
    .poll(() => heading.evaluate(() => CSS.highlights.get("shep-active")?.size))
    .toBe(3);
  await input.fill("Alpha");
  await expect(page.locator(".find-status")).toHaveText("1 of 2");
  await page
    .getByRole("button", { name: "Show quoted history", exact: true })
    .click();
  await expect(page.locator(".find-status")).toHaveText("1 of 3");
  await page.getByRole("button", { name: "Next match", exact: true }).click();
  await expect(page.locator(".find-status")).toHaveText("2 of 3");
  await page.getByRole("button", { name: "Next match", exact: true }).click();
  await expect(page.locator(".find-status")).toHaveText("3 of 3");
  await expect
    .poll(() => heading.evaluate(() => scrollY))
    .toBeGreaterThan(1000);
  await page.setViewportSize({ width: 900, height: 640 });
  await expect(page.locator(".find-status")).toHaveText("3 of 3");
  // Unrelated updates and resizing keep the same browsing context and position.
  await page.getByRole("button", { name: "Flag", exact: true }).click();
  await expect(
    frame(page).getByText("Alpha tail.", { exact: true }),
  ).toBeVisible();
  await expect(page.locator(".find-status")).toHaveText("3 of 3");
  await page
    .getByRole("combobox", { name: "Message format", exact: true })
    .selectOption("Plain text");
  await expect(page.locator(".formatted-frame")).toBeHidden();
  await expect(page.locator(".message-body").first()).toContainText(
    "Plain alternative",
  );
  await expect(page.locator(".find-status")).toHaveText("1 of 2");
  await page
    .getByRole("combobox", { name: "Message format", exact: true })
    .selectOption("Formatted");
  await expect(page.locator(".find-status")).toHaveText("1 of 3");
});

test("hostile mail cannot access the app or network; links use real review controls", async ({
  page,
  context,
}) => {
  const requests: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).hostname.endsWith("example.test"))
      requests.push(request.url());
  });
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await open(page, "Hostile message fixture");
  const body = frame(page).locator("body");
  await expect(body).toContainText("Selectable hostile message.");
  await expect(frame(page).locator("iframe,form,input")).toHaveCount(0);
  await expect(
    frame(page).getByText("Unsafe link", { exact: true }),
  ).not.toHaveAttribute("href");
  await expect
    .poll(() =>
      body.evaluate(() => {
        try {
          return window.parent.document.body.textContent;
        } catch {
          return "isolated";
        }
      }),
    )
    .toBe("isolated");
  expect(await page.evaluate(() => localStorage.getItem("ATTACK"))).toBeNull();
  await expect(frame(page).locator("p.tracker")).toHaveCSS(
    "user-select",
    "text",
  );
  await frame(page)
    .getByRole("link", { name: "Safe link", exact: true })
    .click({ button: "right" });
  const review = page.getByRole("dialog", {
    name: "Message link",
    exact: true,
  });
  await expect(review).toContainText("https://example.test/help");
  await expect(
    review.getByRole("link", { name: "Open link", exact: true }),
  ).toHaveAttribute("rel", "noopener noreferrer");
  await review
    .getByRole("button", { name: "Copy address", exact: true })
    .click();
  await expect(review.getByRole("status")).toHaveText("Address copied.");
  await page.keyboard.press("Escape");
  await page
    .getByRole("button", { name: "Find in message", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Find in message", exact: true })
    .focus();
  await page.keyboard.press("Control+v");
  await expect(
    page.getByRole("textbox", { name: "Find in message", exact: true }),
  ).toHaveValue("https://example.test/help");
  expect(requests).toEqual([]);
});

test("failed preparation is retryable and navigation cancels old worker results", async ({
  page,
}) => {
  await page.route("**/document_worker.ts*", (route) => route.abort());
  await open(page);
  await expect(page.getByRole("alert")).toContainText(
    "formatted reader stopped",
  );
  await expect(page.locator(".message-body").first()).toContainText(
    "Plain alternative",
  );
  await page.unroute("**/document_worker.ts*");
  await page
    .getByRole("button", { name: "Retry formatted message", exact: true })
    .click();
  await expect(
    frame(page).getByRole("heading", { name: "Verification needed" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Close full reader", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Plain message fixture", exact: true })
    .click();
  await expect(page.locator(".message-body").first()).toHaveText(
    "Another plain message.",
  );
  await page.route("**/document_worker.ts*", async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 400));
    await route.continue().catch(() => {});
  });
  await page.getByRole("button", { name: subject, exact: true }).click();
  await page
    .getByRole("button", { name: "Plain message fixture", exact: true })
    .click();
  await expect(page.locator(".message-body").first()).toHaveText(
    "Another plain message.",
  );
  await page.unroute("**/document_worker.ts*");
  await expect(
    page.getByText("Preparing formatted message…", { exact: true }),
  ).toHaveCount(0);
  await expect(page.locator(".formatted-frame")).toHaveCount(0);
});

test("fallback highlights preserve inline layout and can be replaced and closed", async ({
  page,
}) => {
  await page.addInitScript(() =>
    Object.defineProperty(CSS, "highlights", { value: undefined }),
  );
  await page.reload();
  await open(page);
  await expect(
    frame(page).getByRole("heading", { name: "Verification needed" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Find in message", exact: true })
    .click();
  const input = page.getByRole("textbox", {
    name: "Find in message",
    exact: true,
  });
  await input.fill("Alpha across spans");
  await expect(page.locator(".find-status")).toHaveText("1 of 1");
  await expect(frame(page).locator("shep-match[data-active]")).toHaveCount(3);
  await input.fill("café");
  await expect(page.locator(".find-status")).toHaveText("1 of 3");
  await page.getByRole("button", { name: "Match case", exact: true }).click();
  await expect(page.locator(".find-status")).toHaveText("1 of 1");
  await page.getByRole("button", { name: "Close Find", exact: true }).click();
  await expect(frame(page).locator("shep-match")).toHaveCount(0);
  await expect(frame(page).locator("strong")).toHaveText("across");
});

test("a mismatched containing CSP reports a recoverable display error", async ({
  page,
}) => {
  await page.route(/\/$/, async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      headers: {
        ...response.headers(),
        "content-security-policy":
          "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src blob: 'self'; worker-src 'self'; frame-src 'self'",
      },
    });
  });
  await page.reload();
  await open(page);
  await expect(page.getByRole("alert")).toContainText(
    "Could not display formatted mail",
    { timeout: 15000 },
  );
  await expect(page.locator(".message-body").first()).toContainText(
    "Plain alternative",
  );
  await expect(
    page.getByRole("button", { name: "Retry formatted message", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".formatted-frame")).toHaveCount(0);
});

for (const [theme, width, height] of [
  ["light", 1440, 920],
  ["dark", 900, 640],
] as const)
  test(`formatted controls and email layout ${theme} ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height });
    await page
      .getByRole("button", { name: "Preferences", exact: true })
      .click();
    await page
      .getByRole("combobox", { name: "Theme", exact: true })
      .selectOption(theme);
    await page.getByRole("button", { name: "Mail", exact: true }).click();
    await open(page);
    await expect(
      frame(page).getByRole("heading", { name: "Verification needed" }),
    ).toBeVisible();
    await page.screenshot({
      path: `../artifacts/web/formatted-initial-${theme}-${width}.png`,
    });
    await page
      .getByRole("button", { name: "Find in message", exact: true })
      .click();
    await page
      .getByRole("textbox", { name: "Find in message", exact: true })
      .fill("Alpha across spans");
    await expect(page.locator(".find-status")).toHaveText("1 of 1");
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    expect(
      (await page.locator(".formatted-frame").boundingBox())!.width,
    ).toBeGreaterThan(width - 320);
    await page.screenshot({
      path: `../artifacts/web/formatted-${theme}-${width}.png`,
    });
  });
