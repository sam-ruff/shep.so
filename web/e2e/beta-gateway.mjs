// Real Rust middleware and production web build over local HTTPS. Only Google's
// external identity exchange is a fixture in the cfg(test) Rust binary.
import { providerFlows } from "./provider-flows.mjs";
import { chromium, expect } from "@playwright/test";
import assert from "node:assert/strict";
import http from "node:http";
import https from "node:https";
import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// A failed browser/proxy launch must not leave the Rust test waiting forever.
setTimeout(() => process.exit(124), 120_000).unref();
const backendPort = Number(process.argv[2]);
const proxyPort = Number(process.argv[3]);
if (
  ![backendPort, proxyPort].every(
    (p) => Number.isInteger(p) && p > 0 && p < 65536,
  )
) {
  throw new Error("Run through cargo test real_browser_beta_gate -- --ignored");
}
const output = path.join(root, "../artifacts/beta-browser");
await mkdir(output, { recursive: true });
await rm(path.join(output, "result.json"), { force: true });
const upstreamAgent = new http.Agent({ keepAlive: false });
const proxy = https.createServer(
  {
    key: await readFile(
      path.join(root, "../backend/tests/fixtures/https-test-private.pem"),
    ),
    cert: await readFile(
      path.join(root, "../backend/tests/fixtures/https-test-cert.pem"),
    ),
  },
  (request, response) => {
    const upstream = http.request(
      {
        host: "127.0.0.1",
        agent: upstreamAgent,
        port: backendPort,
        path: request.url,
        method: request.method,
        headers: request.headers,
      },
      (incoming) => {
        response.writeHead(incoming.statusCode, incoming.headers);
        incoming.pipe(response);
      },
    );
    upstream.on("error", () => {
      response.writeHead(502);
      response.end();
    });
    request.pipe(upstream);
  },
);
// A browser can leave a speculative TLS connection without an HTTP request.
// Track this fixture's raw sockets as well as Node's HTTP connections.
const proxySockets = new Set();
proxy.on("connection", (socket) => {
  proxySockets.add(socket);
  socket.once("close", () => proxySockets.delete(socket));
});
await new Promise((resolve, reject) => {
  proxy.once("error", reject);
  proxy.listen(proxyPort, "127.0.0.1", resolve);
});
const origin = `https://127.0.0.1:${proxyPort}`;
const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({
  ignoreHTTPSErrors: true,
  viewport: { width: 1440, height: 920 },
});
const page = await context.newPage();
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
let fixtureCode = "owner-fixture";
let callback;
await context.route(`${origin}/auth/start`, async (route) => {
  // Intercept the outgoing redirect at the local edge: Playwright does not
  // route every hop of an already continued redirect chain. Still execute the
  // actual Rust start handler and retain its browser-binding Set-Cookie.
  const started = await route.fetch({ maxRedirects: 0 });
  assert.equal(started.status(), 303);
  const provider = new URL(started.headers()["location"]);
  assert.equal(provider.origin, "https://accounts.google.com");
  assert.equal(provider.pathname, "/o/oauth2/v2/auth");
  const params = provider.searchParams;
  assert.equal(params.get("client_id"), "synthetic-browser-client");
  assert.equal(params.get("code_challenge_method"), "S256");
  assert.equal(params.get("scope"), "openid email");
  assert.equal(params.get("nonce").length, 43);
  assert.equal(params.get("redirect_uri"), `${origin}/auth/callback`);
  callback = `${origin}/auth/callback?${new URLSearchParams({ state: params.get("state"), code: fixtureCode })}`;
  await route.fulfill({
    response: started,
    headers: { ...started.headers(), location: callback },
  });
});
// No requests to actual Google or other external services are allowed.
await context.route("**/*", (route) => {
  const url = new URL(route.request().url());
  if (url.origin === origin) return route.fallback();
  return route.abort("blockedbyclient");
});
const asset = (await readdir(path.join(root, "dist/assets"))).find((name) =>
  name.endsWith(".js"),
);
assert.ok(asset, "Production JavaScript build required");
let result;
try {
  assert.equal(
    (await context.request.get(`${origin}/api/session`)).status(),
    401,
  );
  const printBlocked = await context.request.get(`${origin}/app/print.html`, {
    maxRedirects: 0,
  });
  assert.equal(printBlocked.status(), 303);
  assert.equal(printBlocked.headers()["location"], "/beta");
  const blocked = await context.request.get(`${origin}/app/assets/${asset}`, {
    maxRedirects: 0,
  });
  assert.equal(blocked.status(), 303);
  assert.equal(blocked.headers()["location"], "/beta");
  await page.goto(`${origin}/app/`);
  await expect(page).toHaveURL(`${origin}/beta`);
  await expect(
    page.getByRole("link", { name: "Continue with Google" }),
  ).toBeVisible();
  await page.screenshot({ path: path.join(output, "beta-login.png") });

  fixtureCode = "denied-fixture";
  await page.getByRole("link", { name: "Continue with Google" }).click();
  await expect(
    page.getByText("Sign-in was not accepted.", { exact: false }),
  ).toBeVisible();
  assert.equal(
    (await context.request.get(`${origin}/api/session`)).status(),
    401,
  );
  await page.screenshot({ path: path.join(output, "denied-user.png") });

  fixtureCode = "owner-fixture";
  await page.goto(`${origin}/beta`);
  await page.getByRole("link", { name: "Continue with Google" }).click();
  await expect(page).toHaveURL(`${origin}/app/`);
  await expect(
    page.getByText("owner@example.test", { exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Sign out" })).toBeVisible();
  await expect(
    page.getByText("A little room for good ideas", { exact: true }),
  ).toHaveCount(0);
  const sessionResponse = await context.request.get(`${origin}/api/session`);
  assert.equal(sessionResponse.status(), 200);
  assert.equal(sessionResponse.headers()["cache-control"], "no-store");
  const session = await sessionResponse.json();
  assert.equal(session.email, "owner@example.test");
  assert.equal(
    (await context.request.get(callback)).status(),
    403,
    "One-time callback cannot be replayed",
  );
  const privateAsset = await context.request.get(
    `${origin}/app/assets/${asset}`,
  );
  assert.equal(privateAsset.status(), 200);
  assert.equal(privateAsset.headers()["cache-control"], "no-store");
  const cookie = (await context.cookies()).find(
    (c) => c.name === "__Host-shep_session",
  );
  assert.ok(cookie.secure && cookie.httpOnly && cookie.sameSite === "Lax");
  await page.screenshot({
    path: path.join(output, "allowed-owner-empty-client.png"),
  });

  // Exercise the actual bundled SQLite/WASM worker under production gateway
  // CSP. This is a storage contract, not evidence of selection UI controls.
  const selectionAsset = (await readdir(path.join(root, "dist/assets"))).find(
    (name) => /^selection_worker-.*\.js$/.test(name),
  );
  assert.ok(selectionAsset);
  const selectionResult = await page.evaluate(
    async ({ asset, user }) => {
      const worker = new Worker(`/app/assets/${asset}`, { type: "module" });
      let id = 0;
      const call = (value) =>
        new Promise((resolve, reject) => {
          const expected = ++id;
          worker.onerror = () =>
            reject(Error("Production selection worker failed"));
          worker.onmessage = ({ data }) => {
            if (data.id !== expected || data.phase) return;
            if (data.error) reject(Error(data.error));
            else resolve(data.result);
          };
          worker.postMessage({ id: expected, ...value });
        });
      try {
        await call({ initialize: user });
        const capture = await call({
          command: {
            kind: "capture",
            id: "https-capture",
            revision: 0,
            scope: { folder: "Inbox" },
            all: true,
          },
        });
        const frozen = await call({
          command: {
            kind: "freeze",
            id: "https-capture",
            expected: 0,
            target: "https-review",
          },
        });
        const page = await call({
          command: { kind: "page", id: "https-review", expected: 0 },
        });
        // The real HTTPS worker acquires its journal Web Lock and opens local
        // storage, but an empty frozen selection cannot create runnable work.
        let emptyReviewRefused = false;
        try {
          await call({
            prepareBulk: {
              selection: "https-review",
              expected: 0,
              job: "https-empty",
              action: { kind: "flags", unread: false },
            },
          });
        } catch (error) {
          emptyReviewRefused = String(error).includes(
            "Select at least one message",
          );
        }
        await call({ close: true });
        return { capture, frozen, page, emptyReviewRefused };
      } finally {
        worker.terminate();
      }
    },
    { asset: selectionAsset, user: session.user_id },
  );
  assert.equal(selectionResult.capture.total, 0);
  assert.equal(selectionResult.frozen.frozen, true);
  assert.deepEqual(selectionResult.page.rows, []);
  assert.equal(selectionResult.emptyReviewRefused, true);

  const providerScenarios = await providerFlows(
    page,
    context,
    origin,
    output,
    session,
  );

  // A forged action cannot revoke a legitimate session.
  const forged = await context.request.post(`${origin}/api/logout`, {
    headers: {
      Origin: "https://untrusted.example.test",
      "x-shep-csrf": session.csrf,
    },
  });
  assert.equal(forged.status(), 403);
  assert.equal(
    (await context.request.get(`${origin}/api/session`)).status(),
    200,
  );
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page).toHaveURL(`${origin}/beta`);
  assert.equal(
    (await context.request.get(`${origin}/api/session`)).status(),
    401,
  );
  assert.equal(
    (
      await context.request.get(`${origin}/app/assets/${asset}`, {
        maxRedirects: 0,
      })
    ).status(),
    303,
  );
  assert.deepEqual(errors, []);
  result = {
    passed: true,
    scenarios: [
      ...providerScenarios,
      "anonymous-api-and-assets",
      "login-redirect",
      "denied-user",
      "allowed-owner",
      "replay-rejection",
      "secure-cookie-no-store",
      "production-sqlite-selection-worker",
      "production-durable-journal-empty-review-refusal",
      "csrf-origin",
      "ui-logout-revocation",
    ],
  };
} catch (error) {
  await page.screenshot({ path: path.join(output, "failure.png") });
  throw error;
} finally {
  await browser.close();
  const stopped = new Promise((resolve) => proxy.close(resolve));
  proxy.closeAllConnections();
  for (const socket of proxySockets) socket.destroy();
  upstreamAgent.destroy();
  await stopped;
}

await writeFile(
  path.join(output, "result.json"),
  JSON.stringify(result, null, 2),
);
console.log(
  `PASS: real Rust gateway and production browser, ${result.scenarios.length} beta and mail stages`,
);
