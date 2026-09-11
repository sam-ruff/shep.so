import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';

async function screenshot(page, testInfo, name, fullPage = true) {
  await page.evaluate(() => document.fonts.ready);
  const path = testInfo.outputPath(`${name}.webp`);
  execFileSync('python3', ['-c', 'from PIL import Image; import io, sys; Image.open(io.BytesIO(sys.stdin.buffer.read())).save(sys.argv[1], "WEBP", quality=90)', path], { input: await page.screenshot({ fullPage }) });
  await testInfo.attach(name, { path, contentType: 'image/webp' });
}

async function checkNoOverflow(page) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
}

test('all assets load locally and navigation links resolve to real site sections or documented destinations', async ({ page, request }) => {
  const errors = [];
  const remoteRequests = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => { if (new URL(request.url()).origin !== 'http://127.0.0.1:4178') remoteRequests.push(request.url()); });
  await page.goto('/');
  await expect(page).toHaveTitle('Shep — a little more room for your inbox');
  await page.locator('img[loading="lazy"]').scrollIntoViewIfNeeded();
  for (const image of await page.locator('img').all()) {
    await expect(image).toHaveJSProperty('complete', true);
    expect(await image.evaluate(node => node.naturalWidth)).toBeGreaterThan(0);
  }
  for (const href of await page.locator('a').evaluateAll(links => links.map(link => link.getAttribute('href')))) {
    expect(href).toBeTruthy();
    if (href === '#') continue;
    if (href.startsWith('#')) await expect(page.locator(href)).toHaveCount(1);
    else {
      const url = new URL(href);
      expect(url.protocol).toBe('https:');
      expect(['github.com', 'sam-ruff.github.io']).toContain(url.hostname);
      if (url.hostname === 'sam-ruff.github.io' && url.pathname !== '/shep.so/') {
        expect(existsSync(new URL(`../../docs/${url.pathname.split('/')[2]}.md`, import.meta.url))).toBe(true);
      }
    }
  }
  expect(errors).toEqual([]);
  expect(remoteRequests).toEqual([]);
  expect((await request.get('/not-a-page')).status()).toBe(404);
  expect((await request.get('/app/')).status()).toBe(404);
  expect((await request.get('/beta')).status()).toBe(404);
  expect((await request.post('/')).status()).toBe(405);
});

const platforms = [
  ['linux', 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/149.0.0.0 Safari/537.36', 'Linux x86_64', 0, 'Install on Linux'],
  ['windows', 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/149.0.0.0 Safari/537.36', 'Win32', 0, 'Shep for Windows'],
  ['macos', 'Mozilla/5.0 (Macintosh; Intel Mac OS X 15_0) AppleWebKit/605.1.15 Version/18.0 Safari/605.1.15', 'MacIntel', 0, 'Shep for macOS'],
  ['android', 'Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 Chrome/149.0.0.0 Mobile Safari/537.36', 'Linux armv8l', 5, 'Shep for Android'],
  ['ios', 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1', 'iPhone', 5, 'Shep for iPhone & iPad'],
  ['ios', 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15) AppleWebKit/605.1.15 Version/18.0 Safari/605.1.15', 'MacIntel', 5, 'Shep for iPhone & iPad'],
  ['browser', 'Mozilla/5.0 (X11; CrOS x86_64 16000.0.0) AppleWebKit/537.36 Chrome/149.0.0.0 Safari/537.36', 'Linux x86_64', 0, 'Shep browser beta'],
];
for (const [platform, userAgent, navigatorPlatform, touches, label] of platforms) {
  test(`installation suggestion: ${platform}, ${navigatorPlatform}, touch ${touches}`, async ({ browser }) => {
    const context = await browser.newContext({ userAgent });
    await context.addInitScript(({ platform, touches }) => {
      Object.defineProperty(navigator, 'platform', { get: () => platform });
      Object.defineProperty(navigator, 'maxTouchPoints', { get: () => touches });
      Object.defineProperty(navigator, 'userAgentData', { get: () => undefined });
    }, { platform: navigatorPlatform, touches });
    const page = await context.newPage();
    await page.goto('http://127.0.0.1:4178');
    await expect(page.locator('#suggested-install')).toContainText(label);
    await page.locator('#suggested-install').click();
    await expect(page).toHaveURL(new RegExp(`#${platform}$`));
    await expect(page.locator(`#${platform}`)).toBeInViewport();
    await expect(page.locator('.download-card')).toHaveCount(6);
    await page.getByRole('link', { name: 'Other platforms' }).click();
    await expect(page).toHaveURL(/#download$/);
    await context.close();
  });
}

test('unknown platform keeps all install options accessible', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'userAgent', { get: () => 'Unrecognized browser' });
    Object.defineProperty(navigator, 'platform', { get: () => '' });
    Object.defineProperty(navigator, 'userAgentData', { get: () => undefined });
  });
  await page.goto('/');
  await expect(page.locator('#suggested-install')).toContainText('Choose your platform');
  await page.locator('#suggested-install').click();
  await expect(page).toHaveURL(/#download$/);
});

test('unpublished stores and private beta do not pretend to install or authenticate', async ({ page }, testInfo) => {
  await page.goto('/#download');
  for (const platform of ['android', 'ios', 'browser']) await expect(page.locator(`#${platform} .unavailable`)).toHaveAttribute('aria-disabled', 'true');
  await expect(page.locator('#android')).toContainText('Google Play');
  await expect(page.locator('#ios')).toContainText('App Store');
  await expect(page.locator('#browser')).toContainText('Not deployed');
  await expect(page.locator('#browser')).toContainText('Invite only');
  await expect(page.locator('#browser')).toContainText('Google sign-in');
  await expect(page.locator('#browser')).toContainText('Planned login: shep.so/beta');
  await expect(page.locator('#browser')).not.toContainText('Open web app');
  expect(await page.locator('a[href*="play.google.com"], a[href*="apps.apple.com"], a[href="/app/"], a[href="/beta"], a[href="/beta/"]').count()).toBe(0);
  await expect(page.locator('input, form')).toHaveCount(0);
  await expect(page.locator('#linux')).toContainText('Available from source');
  await expect(page.locator('.source-install')).toContainText('no binary releases are published yet');
  expect(await page.locator('a[href*="/releases/download/"], a[href*="install-release"]').count()).toBe(0);
  await expect(page.locator('.development')).toContainText('static HTML layout');
  await expect(page.locator('.development')).toContainText('The local mail cache is not encrypted');
  await page.locator('#browser').scrollIntoViewIfNeeded();
  await screenshot(page, testInfo, 'private-beta-status', false);
});

test('keyboard skip link, install navigation, source disclosure and appearance control', async ({ page }, testInfo) => {
  await page.goto('/');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('link', { name: 'Skip to content' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('main')).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.locator('#suggested-install')).toBeFocused();
  const target = await page.locator('#suggested-install').getAttribute('href');
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(`http://127.0.0.1:4178/${target}`);
  const disclosure = page.locator('summary');
  await disclosure.focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('details')).toHaveAttribute('open', '');
  await page.locator('#appearance').focus();
  await expect(page.locator('#appearance')).toBeFocused();
  await screenshot(page, testInfo, 'keyboard-focus', false);
});

test('source install copy success and denied-clipboard recovery', async ({ page, context, browserName }) => {
  test.skip(browserName !== 'chromium', 'Clipboard permissions differ by engine; recovery is also covered in the storage-denial flow.');
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto('/');
  await page.locator('summary').click();
  await page.getByRole('button', { name: 'Copy commands' }).click();
  await expect(page.getByRole('status')).toHaveText('Installation commands copied.');
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  expect(copied).toBe('git clone https://github.com/sam-ruff/shep.so.git\ncd shep.so\nbash scripts/install-linux.sh');
  await page.addInitScript(() => Object.defineProperty(navigator, 'clipboard', { get: () => ({ writeText: () => Promise.reject(new Error('Fixture denial')) }) }));
  await page.reload();
  await page.locator('summary').click();
  await page.getByRole('button', { name: 'Copy commands' }).click();
  await expect(page.getByRole('status')).toHaveText('Copy is unavailable. Select and copy the commands above.');
});

test('appearance persists and System follows the OS preference', async ({ page }) => {
  await page.goto('/');
  await page.getByLabel('Appearance').selectOption('dark');
  await expect(page.locator('html')).toHaveCSS('color-scheme', 'dark');
  await page.reload();
  await expect(page.getByLabel('Appearance')).toHaveValue('dark');
  await page.getByLabel('Appearance').selectOption('light');
  await expect(page.locator('html')).toHaveCSS('color-scheme', 'light');
  await page.getByLabel('Appearance').selectOption('system');
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveCSS('color-scheme', 'dark');
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveCSS('color-scheme', 'light');
});

test('blocked browser storage and clipboard leave installation and theme controls usable', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, 'localStorage', { get: () => { throw new Error('Fixture storage denial'); } });
    Object.defineProperty(navigator, 'clipboard', { get: () => undefined });
  });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await page.getByLabel('Appearance').selectOption('dark');
  await expect(page.locator('html')).toHaveCSS('color-scheme', 'dark');
  await page.locator('summary').click();
  await page.getByRole('button', { name: 'Copy commands' }).click();
  await expect(page.getByRole('status')).toContainText('Select and copy');
  await page.getByRole('link', { name: 'Other platforms' }).click();
  await expect(page).toHaveURL(/#download$/);
  expect(errors).toEqual([]);
});

for (const [name, width, height, theme] of [
  ['desktop-light', 1440, 920, 'light'],
  ['desktop-dark', 1440, 920, 'dark'],
  ['compact', 900, 640, 'light'],
  ['tablet', 768, 1024, 'dark'],
  ['phone', 390, 844, 'light'],
  ['small-phone', 320, 640, 'dark'],
]) {
  test(`layout, contrast and semantics: ${name}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height });
    await page.goto('/');
    await page.getByLabel('Appearance').selectOption(theme);
    await checkNoOverflow(page);
    await page.getByRole('link', { name: 'Other platforms' }).click();
    await expect(page.locator('#download-title')).toBeInViewport();
    await expect(page.locator('.download-card')).toHaveCount(6);
    const results = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
    expect(results.violations).toEqual([]);
    await page.locator('summary').click();
    await checkNoOverflow(page);
    await page.getByRole('link', { name: 'Shep home' }).first().click();
    await screenshot(page, testInfo, name);
    await screenshot(page, testInfo, `${name}-hero`, false);
  });
}

test('installation and content remain usable with JavaScript disabled', async ({ browser }) => {
  const context = await browser.newContext({ javaScriptEnabled: false, viewport: { width: 390, height: 844 } });
  const page = await context.newPage();
  await page.goto('http://127.0.0.1:4178');
  await page.getByRole('link', { name: 'Choose your platform' }).click();
  await expect(page).toHaveURL(/#download$/);
  await expect(page.getByRole('link', { name: 'Linux installation guide' })).toBeVisible();
  await page.locator('summary').click();
  await expect(page.locator('#install-command')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Copy commands' })).toBeHidden();
  await context.close();
});
