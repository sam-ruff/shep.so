import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
// Fictional mail only. The newsletter mirrors a reported shape: a transparent
// body, dark authored text and its white page only inside an Outlook comment.
const runtime = readFileSync(
  new URL("../../shared/mail-content/src/document/runtime.js", import.meta.url),
);
const runtimeHash = createHash("sha256").update(runtime).digest("base64");
const profile = "C".repeat(43);
const html = (source: string) =>
  Buffer.from(
    `Content-Type: text/html; charset=utf-8\r\n\r\n${source}`,
  ).toString("base64");
const letter = Array.from(
  { length: 12 },
  (_, i) =>
    `<tr><td style="color:#242424;padding:6px 0">Paragraph ${i + 1} of the harbour walk notes: the tide tables, the ferry times and the lighthouse opening hours for the week.</td></tr>`,
).join("");
const messages = {
  newsletter: {
    subject: "Harbour walks weekly",
    raw: html(
      `<html><head><style>body{margin:0}</style></head><body style="background-color:transparent"><!--[if mso]><table width="100%" bgcolor="#ffffff"><tr><td><![endif]--><table width="100%" cellpadding="0" cellspacing="0" style="max-width:600px;margin:0 auto"><tr><td style="color:#00463a;font-size:26px;font-weight:bold;padding:16px 0">Harbour walks weekly</td></tr>${letter}<tr><td style="color:#00463a">Unsubscribe from the fictional harbour list.</td></tr></table><blockquote style="color:#fafafa">${"Quoted light text from an earlier fictional issue. ".repeat(80)}</blockquote><!--[if mso]></td></tr></table><![endif]--></body></html>`,
    ),
  },
  night: {
    subject: "Night sky club",
    raw: html(
      `<html><head><style>body{background:#0f1720;color:#e5e7eb;padding:20px}h1{color:#a5b4fc}</style></head><body><h1>Night sky club</h1><p>The fictional observatory opens at nine. Bring a red torch.</p></body></html>`,
    ),
  },
  lantern: {
    subject: "Lantern festival notes",
    raw: html(
      `<html><body style="background:transparent"><h1 style="color:#fef3c7">Lantern festival</h1><p style="color:#f5f5f5">Pale text written for a dark page by a fictional sender.</p><p style="color:#e4e4e7">The parade starts at the fictional market square.</p></body></html>`,
    ),
  },
  unstyled: {
    subject: "Plain club minutes",
    raw: html(
      `<p>Minutes of the fictional allotment club.</p><p>No colours are set in this message.</p>`,
    ),
  },
};
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
  await page.route("**/seed-canvas", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: "<!doctype html><title>Isolated cache setup</title>",
    }),
  );
  await page.goto("/seed-canvas");
  // Fixture setup precedes mounting the production application; every
  // subsequent action uses actual controls.
  await page.evaluate(
    async ({ profile, messages }) => {
      const { BrowserStore } = await import("/src/storage.ts");
      const store = await BrowserStore.open(profile);
      const account = {
        id: "fixture",
        name: "Canvas fixture",
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
      let offset = 0;
      for (const [id, { subject, raw }] of Object.entries(messages)) {
        const core = {
          id,
          account_id: account.id,
          remote_id: id,
          folder: "INBOX",
          sender: "Fictional sender <sender@example.test>",
          recipient: account.email,
          subject,
          preview: subject,
          timestamp: 1788692400 - offset++,
          unread: false,
          starred: false,
          attachment_count: 0,
        };
        changes.push(
          { store: "mail", key: id, value: { core, text: subject } },
          { store: "raw", key: id, value: raw },
        );
      }
      await store.commit(changes);
      store.close();
    },
    { profile, messages },
  );
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
async function theme(page: Page, value: "light" | "dark") {
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await page
    .getByRole("combobox", { name: "Theme", exact: true })
    .selectOption(value);
  await page.getByRole("button", { name: "Mail", exact: true }).click();
}
async function open(page: Page, subject: string) {
  const close = page.getByRole("button", {
    name: "Close full reader",
    exact: true,
  });
  if (await close.isVisible()) await close.click();
  await page.getByRole("button", { name: subject, exact: true }).dblclick();
  await expect(
    page.getByRole("heading", { name: subject, level: 1 }),
  ).toBeVisible();
}
// Background painted behind the text and the text's contrast against it.
async function reading(page: Page, text: string) {
  return frame(page)
    .getByText(text)
    .first()
    .evaluate((node) => {
      const parse = (value: string) => {
        const [r, g, b, a = 1] = value.match(/[\d.]+/g)!.map(Number);
        return { r, g, b, a };
      };
      const luminance = ({ r, g, b }: { r: number; g: number; b: number }) => {
        const c = (v: number) => {
          v /= 255;
          return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
        };
        return 0.2126 * c(r) + 0.7152 * c(g) + 0.0722 * c(b);
      };
      let behind = parse("rgba(0, 0, 0, 0)");
      for (
        let element: Element | null = node;
        element && behind.a < 1;
        element = element.parentElement
      )
        behind = parse(getComputedStyle(element).backgroundColor);
      const fore = parse(getComputedStyle(node).color);
      const [hi, lo] = [luminance(fore), luminance(behind)].sort(
        (a, b) => b - a,
      );
      return {
        background: `rgb(${behind.r}, ${behind.g}, ${behind.b})`,
        contrast: (hi + 0.05) / (lo + 0.05),
        scheme: getComputedStyle(document.documentElement).colorScheme,
      };
    });
}
async function reported(page: Page) {
  const canvas = page.locator(".formatted-frame");
  await expect(canvas).toHaveAttribute("data-canvas", /light|dark/);
  return {
    scheme: await canvas.getAttribute("data-canvas"),
    frame: await canvas.evaluate((n) => getComputedStyle(n).backgroundColor),
    surround: await page
      .locator(".formatted-viewport")
      .evaluate((n) => getComputedStyle(n).backgroundColor),
  };
}

for (const [mode, width, height] of [
  ["light", 1440, 920],
  ["dark", 1440, 920],
  ["light", 390, 844],
  ["dark", 390, 844],
] as const)
  test(`transparent mail gets a readable canvas in ${mode} at ${width}`, async ({
    page,
  }) => {
    // Preferences sit in the sidebar, which the phone layout collapses.
    await page.setViewportSize({ width: 1440, height: 920 });
    await seed(page);
    await theme(page, mode);
    await page.setViewportSize({ width, height });
    const shot = (name: string) =>
      page.screenshot({
        path: `../artifacts/web/canvas-${name}-${mode}-${width}.png`,
      });

    await open(page, messages.newsletter.subject);
    await expect(
      frame(page).getByText("Paragraph 1 of the harbour walk notes"),
    ).toBeVisible();
    const paper = await reading(page, "Paragraph 1 of the harbour walk notes");
    expect(paper.background).toBe("rgb(255, 255, 255)");
    expect(paper.scheme).toBe("light");
    expect(paper.contrast).toBeGreaterThan(7);
    expect(
      (await reading(page, "Unsubscribe from the fictional")).contrast,
    ).toBeGreaterThan(7);
    expect(await reported(page)).toEqual({
      scheme: "light",
      frame: "rgb(255, 255, 255)",
      surround: "rgb(255, 255, 255)",
    });
    await shot("newsletter");
    // The choice is made once: showing a long light-text quote or scrolling
    // the document keeps the paper it was read on.
    const show = page.getByRole("button", {
      name: "Show quoted history",
      exact: true,
    });
    if (await show.isVisible()) {
      await show.click();
      await expect(frame(page).locator("blockquote")).toHaveCount(1);
    }
    await frame(page)
      .locator("body")
      .evaluate(() => window.scrollTo(0, document.body.scrollHeight));
    expect(
      (await reading(page, "Paragraph 12 of the harbour walk notes"))
        .background,
    ).toBe("rgb(255, 255, 255)");
    expect((await reported(page)).scheme).toBe("light");

    await open(page, messages.night.subject);
    await expect(frame(page).getByText("Bring a red torch.")).toBeVisible();
    const night = await reading(page, "Bring a red torch.");
    expect(night.background).toBe("rgb(15, 23, 32)");
    expect(night.contrast).toBeGreaterThan(7);
    expect(await reported(page)).toEqual({
      scheme: "dark",
      frame: "rgb(15, 23, 32)",
      surround: "rgb(15, 23, 32)",
    });
    await shot("authored-dark");

    await open(page, messages.lantern.subject);
    await expect(frame(page).getByText("Pale text written")).toBeVisible();
    const lantern = await reading(page, "Pale text written");
    expect(lantern.background).toBe("rgb(24, 24, 27)");
    expect(lantern.scheme).toBe("dark");
    expect(lantern.contrast).toBeGreaterThan(7);
    expect((await reported(page)).surround).toBe("rgb(24, 24, 27)");
    await shot("light-text");

    // Unstyled mail follows the theme through its default text colour.
    await open(page, messages.unstyled.subject);
    await expect(frame(page).getByText("No colours are set")).toBeVisible();
    const plain = await reading(page, "No colours are set");
    expect(plain.background).toBe(
      mode === "dark" ? "rgb(24, 24, 27)" : "rgb(255, 255, 255)",
    );
    expect(plain.contrast).toBeGreaterThan(7);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  });

test("a theme change rechooses the canvas for unstyled mail only", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 920 });
  await seed(page);
  await theme(page, "light");
  await open(page, messages.unstyled.subject);
  await expect(frame(page).getByText("No colours are set")).toBeVisible();
  expect((await reported(page)).scheme).toBe("light");
  await theme(page, "dark");
  await expect(page.locator(".formatted-frame")).toHaveAttribute(
    "data-canvas",
    "dark",
  );
  const plain = await reading(page, "No colours are set");
  expect(plain.background).toBe("rgb(24, 24, 27)");
  expect(plain.contrast).toBeGreaterThan(7);
  await open(page, messages.newsletter.subject);
  await expect(
    frame(page).getByText("Paragraph 1 of the harbour walk notes"),
  ).toBeVisible();
  await theme(page, "light");
  await theme(page, "dark");
  expect(
    (await reading(page, "Paragraph 1 of the harbour walk notes")).background,
  ).toBe("rgb(255, 255, 255)");
});
