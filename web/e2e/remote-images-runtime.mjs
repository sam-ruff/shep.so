// Runs the shared reader runtime from shared/mail-content in Chromium and
// WebKit. Remote images stay unloaded until the host delivers permitted bytes;
// arrivals then keep the reading position, selection and Find highlights.
// The bridge calls stand in for the Flutter host; no external host is used.
import { chromium, webkit } from "playwright";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const out = path.join(root, "artifacts/web/remote-images");
await mkdir(out, { recursive: true });
// 120×300 lossless WebP.
const WEBP = "UklGRiYAAABXRUJQVlA4TBkAAAAvd8BKAAdQqt61tP8BIUHC//cmI/qf9r8JAA==";
const paragraphs = Array.from(
  { length: 40 },
  (_, i) =>
    `<p id="p${i}">Paragraph ${i} keeps enough reading text to wrap across the narrow test viewport.</p>`,
).join("");
const mime = `Content-Type: text/html\r\n\r\n<body background="https://images.example.test/body.webp"><style>.hero{background:url("https://images.example.test/css.webp")}</style><div class="hero">Hero</div><img id="top" src="https://images.example.test/top.webp" alt="Top"><p id="styled" style="background-image:url(https://images.example.test/inline.webp)">Styled</p>${paragraphs}<img id="bottom" src="https://images.example.test/bottom.webp" alt="Bottom"><p id="last">Last line</p></body>`;
const generation = "remote-images-fixture";
const prepared = JSON.parse(
  execFileSync(
    "cargo",
    [
      "run",
      "--quiet",
      "--locked",
      "-p",
      "shep-mail-content",
      "--example",
      "prepare",
      "--",
      JSON.stringify({ generation, remote_placeholders: true }),
    ],
    {
      cwd: root,
      input: Buffer.from(mime),
      env: {
        ...process.env,
        CARGO_TARGET_DIR:
          process.env.CARGO_TARGET_DIR ??
          path.join(root, "artifacts/target-root"),
      },
    },
  ).toString(),
);
const key = Object.fromEntries(
  prepared.remote_images.map((image) => [
    new URL(image.url).pathname.slice(1).replace(".webp", ""),
    image.key,
  ]),
);
assert.deepEqual(Object.keys(key).sort(), [
  "body",
  "bottom",
  "css",
  "inline",
  "top",
]);
assert.ok(!prepared.document.includes("images.example.test"));
const image = { bytes: WEBP, width: 120, height: 300 };

async function open(context) {
  const page = await context.newPage();
  await page.addInitScript(() => {
    window.__messages = [];
    window.ShepReader = {
      postMessage: (value) => window.__messages.push(JSON.parse(value)),
    };
  });
  await page.goto("https://shep-reader.invalid/");
  await page.waitForFunction(() =>
    window.__messages.some((message) => message.type === "ready"),
  );
  return page;
}

async function deliver(page, images, expected = generation) {
  const before = await page.evaluate(
    () => window.__messages.filter((m) => m.type === "images").length,
  );
  await page.evaluate(
    ([images, generation]) =>
      window.shepReaderCommand({ type: "images", generation, images }),
    [images, expected],
  );
  return before;
}

async function shown(page, before) {
  await page.waitForFunction(
    (count) =>
      window.__messages.filter((m) => m.type === "images").length > count,
    before,
  );
}

async function anchor(page, id) {
  return page.evaluate(
    (id) => document.getElementById(id).getBoundingClientRect().top,
    id,
  );
}

async function run(engine, name) {
  const browser = await engine.launch({ headless: true });
  const context = await browser.newContext({
    viewport: { width: 360, height: 480 },
  });
  const external = [];
  // WebKit routes blob: loads too; those stay local to the document.
  await context.route(
    (url) => url.protocol !== "blob:",
    (route) => {
      const url = route.request().url();
      if (url === "https://shep-reader.invalid/")
        return route.fulfill({
          status: 200,
          contentType: "text/html",
          body: prepared.document,
        });
      external.push(url);
      return route.abort();
    },
  );
  try {
    const page = await open(context);
    const state = () =>
      page.evaluate(() => ({
        top: document.getElementById("top").getAttribute("src"),
        body: document.body.getAttribute("background"),
        hero: getComputedStyle(document.querySelector(".hero")).backgroundImage,
        styled: getComputedStyle(document.getElementById("styled"))
          .backgroundImage,
      }));
    assert.deepEqual(await state(), {
      top: null,
      body: null,
      hero: "none",
      styled: "none",
    });
    // The document itself can reach nothing, whatever the sender wrote.
    const probes = await page.evaluate(async () => {
      const fetched = await fetch("https://images.example.test/probe").then(
        () => "loaded",
        () => "blocked",
      );
      const loaded = await new Promise((resolve) => {
        const img = new Image();
        img.onload = () => resolve("loaded");
        img.onerror = () => resolve("blocked");
        img.src = "https://images.example.test/probe.webp";
      });
      return [fetched, loaded];
    });
    assert.deepEqual(probes, ["blocked", "blocked"]);

    await page.evaluate(() => window.scrollTo(0, 700));
    const reading = await page.evaluate(() => {
      for (const node of document.querySelectorAll("p[id^=p]")) {
        if (node.getBoundingClientRect().bottom > 0) return node.id;
      }
      return null;
    });
    assert.ok(reading);
    const range = await page.evaluate(() => {
      const range = document.createRange();
      range.selectNodeContents(document.getElementById("p20"));
      getSelection().removeAllRanges();
      getSelection().addRange(range);
      return getSelection().toString();
    });
    const content = await page.evaluate(() =>
      window.__messages.filter((m) => m.type === "content").at(-1),
    );
    const block = content.blocks.findIndex((text) =>
      text.includes("Paragraph 25"),
    );
    const offset = content.blocks[block].indexOf("Paragraph 25");
    await page.evaluate(
      ([generation, layout, block, offset]) =>
        window.shepReaderCommand({
          type: "highlight",
          generation,
          layout,
          revision: 1,
          hits: [{ block, start: offset, end: offset + 12 }],
          active: 0,
          jump: false,
        }),
      [generation, content.layout, block, offset],
    );
    const highlighted = () =>
      page.evaluate(
        () =>
          (window.CSS?.highlights?.get("shep-active")?.size ?? 0) +
          document.querySelectorAll("shep-match[data-active]").length,
      );
    assert.equal(await highlighted(), 1);
    const before = {
      top: await anchor(page, reading),
      y: await page.evaluate(() => scrollY),
    };

    // Unrequested keys and another document's generation change nothing.
    await deliver(page, { ["f".repeat(64)]: image });
    await deliver(page, { [key.top]: image }, "another-generation");
    await page.waitForTimeout(300);
    assert.equal((await state()).top, null);

    const count = await deliver(page, { [key.top]: image });
    await shown(page, count);
    const after = await page.evaluate(() => ({
      src: document.getElementById("top").getAttribute("src"),
      height: document.getElementById("top").naturalHeight,
      y: scrollY,
      selected: getSelection().toString(),
    }));
    assert.match(after.src, /^blob:/);
    assert.equal(after.height, 300);
    assert.ok(after.y - before.y > 200, `${name} moved ${after.y - before.y}`);
    assert.ok(Math.abs((await anchor(page, reading)) - before.top) <= 1);
    assert.equal(after.selected, range);
    assert.equal(await highlighted(), 1);
    await page.screenshot({ path: path.join(out, `${name}-arrived.png`) });

    // An image below the reading position does not move it.
    const y = await page.evaluate(() => scrollY);
    const top = await anchor(page, reading);
    const below = await deliver(page, { [key.bottom]: image });
    await shown(page, below);
    assert.equal(await page.evaluate(() => scrollY), y);
    assert.ok(Math.abs((await anchor(page, reading)) - top) <= 1);

    const rest = await deliver(page, {
      [key.body]: image,
      [key.css]: image,
      [key.inline]: image,
    });
    await shown(page, rest);
    const filled = await state();
    assert.match(filled.body, /^blob:/);
    assert.match(filled.hero, /blob:/);
    assert.match(filled.styled, /blob:/);
    const blob = filled.top;
    await page.evaluate(() => window.shepReaderDispose());
    assert.equal(
      await page.evaluate(
        (url) =>
          fetch(url).then(
            () => "available",
            () => "revoked",
          ),
        blob,
      ),
      "revoked",
    );

    // A reader at the start stays at the start.
    const start = await open(context);
    const first = await deliver(start, { [key.top]: image });
    await shown(start, first);
    assert.equal(await start.evaluate(() => scrollY), 0);
    await start.screenshot({ path: path.join(out, `${name}-start.png`) });
    assert.deepEqual(external, []);
    console.log(`${name}: remote image runtime checks passed`);
  } finally {
    await browser.close();
  }
}

for (const [engine, name] of [
  [chromium, "chromium"],
  [webkit, "webkit"],
]) {
  await run(engine, name);
}
