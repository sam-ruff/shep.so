import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';

const origin = 'http://127.0.0.1:4178';
const commands = {
  linux: 'curl -fsSL https://raw.githubusercontent.com/sam-ruff/shep.so/main/scripts/install-release-linux.sh | bash',
  macos: 'curl -fsSL https://raw.githubusercontent.com/sam-ruff/shep.so/main/scripts/install-release-macos.sh | bash',
  windows: '& ([scriptblock]::Create((Invoke-RestMethod https://raw.githubusercontent.com/sam-ruff/shep.so/main/scripts/install-release-windows.ps1)))',
};

async function screenshot(page, testInfo, name, fullPage = true) {
  await page.evaluate(() => document.fonts.ready);
  const path = testInfo.outputPath(`${name}.webp`);
  execFileSync('python3', ['-c', 'from PIL import Image; import io, sys; Image.open(io.BytesIO(sys.stdin.buffer.read())).save(sys.argv[1], "WEBP", quality=90)', path], { input: await page.screenshot({ fullPage }) });
  await testInfo.attach(name, { path, contentType: 'image/webp' });
}

async function checkNoOverflow(page) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
}

async function contextFor(browser, userAgent, platform, touches, options = {}) {
  const context = await browser.newContext({ userAgent, ...options });
  await context.addInitScript(({ platform, touches }) => {
    // Configurable, because a new tab can run the init script twice.
    Object.defineProperty(navigator, 'platform', { get: () => platform, configurable: true });
    Object.defineProperty(navigator, 'maxTouchPoints', { get: () => touches, configurable: true });
    Object.defineProperty(navigator, 'userAgentData', { get: () => undefined, configurable: true });
  }, { platform, touches });
  return context;
}

test('all assets load locally and links resolve to real sections, the demo or documented destinations', async ({ page, request }) => {
  const errors = [];
  const remoteRequests = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => { if (new URL(request.url()).origin !== origin) remoteRequests.push(request.url()); });
  await page.goto('/');
  await expect(page).toHaveTitle('Shep: a little more room for your inbox');
  for (const source of await page.locator('img').evaluateAll(images => images.map(image => image.getAttribute('src')))) {
    const response = await request.get(`/${source}`);
    expect(response.status(), source).toBe(200);
    expect(response.headers()['content-type'], source).toMatch(/^image\//);
  }
  for (const image of await page.locator('img:visible').all()) {
    await image.scrollIntoViewIfNeeded();
    await expect(image).toHaveJSProperty('complete', true);
    expect(await image.evaluate(node => node.naturalWidth)).toBeGreaterThan(0);
  }
  for (const href of await page.locator('a').evaluateAll(links => links.map(link => link.getAttribute('href')))) {
    expect(href).toBeTruthy();
    if (href === '#') continue;
    if (href.startsWith('#')) await expect(page.locator(href)).toHaveCount(1);
    else if (href === 'demo/') expect((await request.get('/demo/')).status()).toBe(200);
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
  expect((await request.get('/beta')).status()).toBe(404);
  expect((await request.post('/')).status()).toBe(405);
  await expect(page.getByText(/in development|not published|private beta/i)).toHaveCount(0);
});

const platforms = [
  ['linux', 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/149.0.0.0 Safari/537.36', 'Linux x86_64', 0, 'Linux'],
  ['linux', 'Mozilla/5.0 (X11; CrOS x86_64 16000.0.0) AppleWebKit/537.36 Chrome/149.0.0.0 Safari/537.36', 'Linux x86_64', 0, 'Linux'],
  ['windows', 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/149.0.0.0 Safari/537.36', 'Win32', 0, 'Windows'],
  ['macos', 'Mozilla/5.0 (Macintosh; Intel Mac OS X 15_0) AppleWebKit/605.1.15 Version/18.0 Safari/605.1.15', 'MacIntel', 0, 'macOS'],
  ['android', 'Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 Chrome/149.0.0.0 Mobile Safari/537.36', 'Linux armv8l', 5, 'Android'],
  ['ios', 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1', 'iPhone', 5, 'iPhone and iPad'],
  ['ios', 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15) AppleWebKit/605.1.15 Version/18.0 Safari/605.1.15', 'MacIntel', 5, 'iPhone and iPad'],
];
for (const [platform, userAgent, navigatorPlatform, touches, name] of platforms) {
  test(`Get Shep opens the detected platform: ${platform}, ${userAgent.match(/\(([^)]+)/)[1]}, ${navigatorPlatform}, touch ${touches}`, async ({ browser }) => {
    const context = await contextFor(browser, userAgent, navigatorPlatform, touches);
    const page = await context.newPage();
    await page.goto(origin);
    const mobile = platform === 'android' || platform === 'ios';
    await expect(page.locator('#suggested-install')).toHaveText(mobile ? 'Try Shep in your browser' : `Get Shep for ${name}`);
    await expect(page.locator('#suggested-install')).toHaveAttribute('href', mobile ? 'demo/' : '#download');
    await page.getByRole('link', { name: 'Get Shep', exact: true }).click();
    await expect(page).toHaveURL(/#download$/);
    await expect(page.locator(`#${platform}`)).toBeVisible();
    await expect(page.locator('.install-panel:visible')).toHaveCount(1);
    await expect(page.getByRole('tab', { name, exact: true })).toHaveAttribute('aria-selected', 'true');
    await expect(page.getByRole('tab')).toHaveCount(5);
    await expect(page.locator('#demo-notice')).toBeVisible({ visible: mobile });
    await page.goto(`${origin}/demo/`);
    await expect(page.getByRole('dialog', { name: 'This demo is for desktop only.' })).toHaveCount(mobile ? 1 : 0);
    if (!mobile) await expect(page.locator('#demo-frame')).toBeFocused();
    await context.close();
  });
}

const mobileNotice = 'Shep has a mobile app designed for a great experience on mobile.';
const android = ['Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 Chrome/149.0.0.0 Mobile Safari/537.36', 'Linux armv8l', 5];
const iphone = ['Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1', 'iPhone', 5];
const axeTags = ['wcag2a', 'wcag2aa', 'wcag21aa'];

// The demo has no reader pane at phone widths, so searching shows it still takes input.
async function searchDemo(demo) {
  await demo.getByRole('textbox', { name: 'Search conversations' }).fill('coffee');
  await expect(demo.getByRole('article')).toHaveCount(1);
  await expect(demo.getByRole('button', { name: 'Coffee on Thursday?', exact: true })).toBeVisible();
}

test('mobile home: the demo notice covers the embedded demo without taking focus, and the keyboard continues', async ({ browser }, testInfo) => {
  const context = await contextFor(browser, ...android, { viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: testInfo.project.name !== 'firefox' });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(origin);
  const notice = page.getByRole('region', { name: 'This demo is for desktop only.' });
  await expect(notice).toBeVisible();
  await expect(notice).toContainText(mobileNotice);
  await expect(page.frameLocator('#demo-frame').getByText('Coffee on Thursday?')).toBeAttached();
  expect(await page.evaluate(() => [window.scrollY, document.activeElement === document.body])).toEqual([0, true]);
  // The card sits over the frame, inside the demo window.
  const [card, frame] = await Promise.all([page.locator('.notice-card').boundingBox(), page.locator('#demo-frame').boundingBox()]);
  expect(card.x >= frame.x && card.y >= frame.y && card.x + card.width <= frame.x + frame.width && card.y + card.height <= frame.y + frame.height).toBe(true);
  await checkNoOverflow(page);
  expect((await new AxeBuilder({ page }).withTags(axeTags).analyze()).violations).toEqual([]);
  await screenshot(page, testInfo, 'mobile-home-notice-light', false);
  await page.locator('#demo').scrollIntoViewIfNeeded();
  await screenshot(page, testInfo, 'mobile-home-notice-light-demo', false);

  // Tab from the Full screen link skips the inert frame and reaches the notice.
  await page.getByRole('link', { name: /^Full screen/ }).focus();
  await page.keyboard.press('Tab');
  const proceed = notice.getByRole('button', { name: 'Continue to the demo' });
  await expect(proceed).toBeFocused();
  expect(await page.evaluate(() => document.querySelector('#demo-frame').inert)).toBe(true);
  await page.keyboard.press('Enter');
  await expect(notice).toBeHidden();
  await expect(page.locator('#demo-frame')).toBeFocused();
  await searchDemo(page.frameLocator('#demo-frame'));
  expect(errors).toEqual([]);
  await context.close();
});

test('small phone home in dark: the notice fits, ignores unrelated Escape and closes by touch or Escape', async ({ browser }, testInfo) => {
  const context = await contextFor(browser, ...iphone, { viewport: { width: 320, height: 640 }, hasTouch: true, isMobile: testInfo.project.name !== 'firefox' });
  const page = await context.newPage();
  await page.goto(origin);
  await page.getByLabel('Appearance').selectOption('dark');
  await expect(page.locator('html')).toHaveCSS('color-scheme', 'dark');
  const notice = page.getByRole('region', { name: 'This demo is for desktop only.' });
  await expect(notice).toBeVisible();
  await checkNoOverflow(page);
  expect((await new AxeBuilder({ page }).withTags(axeTags).analyze()).violations).toEqual([]);
  await page.locator('#demo').scrollIntoViewIfNeeded();
  await screenshot(page, testInfo, 'small-phone-home-notice-dark', false);

  // Escape elsewhere on the page leaves the notice alone.
  await page.getByLabel('Appearance').focus();
  await page.keyboard.press('Escape');
  await expect(notice).toBeVisible();
  const proceed = notice.getByRole('button', { name: 'Continue to the demo' });
  expect((await proceed.boundingBox()).height).toBeGreaterThanOrEqual(44);
  await proceed.focus();
  await page.keyboard.press('Escape');
  await expect(notice).toBeHidden();
  await expect(page.locator('#demo-frame')).toBeFocused();

  // Changing the appearance reloads the demo but a dismissed notice stays dismissed for this page view.
  await page.getByLabel('Appearance').selectOption('light');
  await expect(notice).toBeHidden();
  await page.reload();
  await expect(notice).toBeVisible();
  await notice.getByRole('button', { name: 'Continue to the demo' }).tap();
  await expect(notice).toBeHidden();
  const demo = page.frameLocator('#demo-frame');
  await demo.getByRole('textbox', { name: 'Search conversations' }).tap();
  await searchDemo(demo);
  await context.close();
});

test('small phone home with enlarged text: the notice scrolls so Continue stays reachable', async ({ browser }, testInfo) => {
  const context = await contextFor(browser, ...android, { viewport: { width: 320, height: 640 }, hasTouch: true, isMobile: testInfo.project.name !== 'firefox' });
  const page = await context.newPage();
  await page.goto(origin);
  const notice = page.locator('#demo-notice');
  await expect(notice).toBeVisible();
  // The site sizes text in px, so doubling it matches a phone's larger-text setting.
  await page.evaluate(() => {
    for (const element of document.querySelectorAll('.notice-card, .notice-card *')) {
      element.style.fontSize = `${parseFloat(getComputedStyle(element).fontSize) * 2}px`;
    }
  });
  await page.locator('#demo').scrollIntoViewIfNeeded();
  const overflow = await notice.evaluate(element => [getComputedStyle(element).overflowY, element.scrollHeight > element.clientHeight]);
  expect(overflow).toEqual(['auto', true]);
  // Before any scrolling, the heading starts inside the stage rather than above its top.
  const start = await page.evaluate(() => ({
    scrollTop: document.querySelector('#demo-notice').scrollTop,
    heading: document.querySelector('#demo-notice-title').getBoundingClientRect().top,
    stage: document.querySelector('.demo-stage').getBoundingClientRect().top,
  }));
  expect(start.scrollTop).toBe(0);
  expect(start.heading, JSON.stringify(start)).toBeGreaterThanOrEqual(start.stage);
  const proceed = page.getByRole('button', { name: 'Continue to the demo' });
  const inside = async () => {
    const [button, stage] = await Promise.all([proceed.boundingBox(), page.locator('.demo-stage').boundingBox()]);
    return button.y >= stage.y && button.y + button.height <= stage.y + stage.height;
  };
  expect(await inside()).toBe(false);
  if (testInfo.project.name !== 'webkit') {
    // WebKit's mobile emulation has no wheel input; its overflow check above covers the same rule.
    const stage = await page.locator('.demo-stage').boundingBox();
    await page.mouse.move(stage.x + stage.width / 2, stage.y + stage.height / 2);
    await page.mouse.wheel(0, 2000);
    await expect.poll(() => notice.evaluate(element => element.scrollTop)).toBeGreaterThan(0);
    await expect.poll(inside).toBe(true);
    await checkNoOverflow(page);
    await screenshot(page, testInfo, 'small-phone-home-notice-large-text-scrolled', false);
    await proceed.click();
    await expect(notice).toBeHidden();
    await expect(page.locator('#demo-frame')).toBeFocused();
  }
  await context.close();
});

// Chromium only: raw CDP touch events, which the browser turns into a scroll gesture.
// Input.synthesizeScrollGesture does not scroll even ordinary page content in headless Chromium.
async function touchSwipe(page, x, y, distance) {
  const session = await page.context().newCDPSession(page);
  const steps = 12;
  await session.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: Math.round(x), y: Math.round(y) }] });
  for (let step = 1; step <= steps; step++) {
    await session.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: Math.round(x), y: Math.round(y - step * distance / steps) }] });
    await page.waitForTimeout(16);
  }
  await session.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
  await session.detach();
}

test('phone home at default text: swiping or wheeling over the notice still scrolls the page', async ({ browser }, testInfo) => {
  const engine = testInfo.project.name;
  test.skip(engine === 'webkit', 'WebKit mobile emulation supports neither wheel nor touch gestures.');
  const context = await contextFor(browser, ...android, { viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: engine !== 'firefox' });
  const page = await context.newPage();
  await page.goto(origin);
  await expect(page.locator('#demo-notice')).toBeVisible();
  // Put the stage top 500 px down the viewport, leaving page content above it and room to scroll.
  const stageLow = async () => {
    await page.locator('.demo-stage').evaluate(element => window.scrollTo(0, element.getBoundingClientRect().top + window.scrollY - 500));
    await expect.poll(() => page.evaluate(() => document.querySelector('.demo-stage').getBoundingClientRect().top)).toBeCloseTo(500, 0);
  };
  const scrollY = () => page.evaluate(() => window.scrollY);
  const overNotice = () => page.evaluate(() => document.elementFromPoint(195, 780)?.closest('#demo-notice') !== null);
  await stageLow();
  expect(await overNotice()).toBe(true);
  expect(await page.locator('#demo-notice').evaluate(element => element.scrollHeight <= element.clientHeight)).toBe(true);
  if (engine === 'chromium') {
    // Control: the same drag over page content above the stage scrolls the page.
    let before = await scrollY();
    await touchSwipe(page, 195, 450, 240);
    await expect.poll(scrollY).toBeGreaterThan(before);
    await stageLow();
    before = await scrollY();
    await touchSwipe(page, 195, 780, 240);
    await expect.poll(scrollY).toBeGreaterThan(before);
    await stageLow();
  }
  const before = await scrollY();
  await page.mouse.move(195, 780);
  await page.mouse.wheel(0, 300);
  await expect.poll(scrollY).toBeGreaterThan(before);
  await context.close();
});

test('mobile full screen: a modal notice opens first and Escape returns to the demo', async ({ browser }, testInfo) => {
  const context = await contextFor(browser, ...android, { viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: testInfo.project.name !== 'firefox' });
  const page = await context.newPage();
  const errors = [];
  context.on('weberror', error => errors.push(error.error().message));
  await page.goto(origin);
  await page.getByRole('button', { name: 'Continue to the demo' }).click();
  const [tab] = await Promise.all([context.waitForEvent('page'), page.getByRole('link', { name: /^Full screen/ }).click()]);
  await expect(tab).toHaveURL(`${origin}/demo/`);
  const dialog = tab.getByRole('dialog', { name: 'This demo is for desktop only.' });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText(mobileNotice);
  const proceed = dialog.getByRole('button', { name: 'Continue to the demo' });
  await expect(proceed).toBeFocused();
  expect(await tab.evaluate(() => document.querySelector('#demo-notice').matches(':modal'))).toBe(true);
  await tab.keyboard.press('Tab');
  expect(await tab.evaluate(() => document.activeElement !== document.querySelector('#demo-frame'))).toBe(true);
  await checkNoOverflow(tab);
  expect((await new AxeBuilder({ page: tab }).withTags(axeTags).analyze()).violations).toEqual([]);
  await screenshot(tab, testInfo, 'mobile-full-screen-notice-light', false);
  await proceed.focus();
  await tab.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(tab.locator('#demo-frame')).toBeFocused();
  const size = await tab.evaluate(() => {
    const box = document.querySelector('#demo-frame').getBoundingClientRect();
    return [box.x, box.y, box.width === innerWidth, box.height === innerHeight];
  });
  expect(size).toEqual([0, 0, true, true]);
  await searchDemo(tab.frameLocator('#demo-frame'));
  expect(errors).toEqual([]);
  await context.close();
});

for (const [name, query, colorScheme] of [['requested', '?appearance=dark', 'light'], ['system', '', 'dark']]) {
  test(`small phone full screen follows the ${name} dark appearance and closes with its button`, async ({ browser }, testInfo) => {
    const context = await contextFor(browser, ...iphone, { viewport: { width: 320, height: 640 }, hasTouch: true, isMobile: testInfo.project.name !== 'firefox', colorScheme });
    const page = await context.newPage();
    await page.goto(`${origin}/demo/${query}`);
    const dialog = page.getByRole('dialog', { name: 'This demo is for desktop only.' });
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveCSS('background-color', 'rgb(28, 26, 32)');
    await expect(dialog.getByRole('heading')).toHaveCSS('color', 'rgb(244, 241, 248)');
    await checkNoOverflow(page);
    expect((await new AxeBuilder({ page }).withTags(axeTags).analyze()).violations).toEqual([]);
    await screenshot(page, testInfo, `small-phone-full-screen-notice-dark-${name}`, false);
    await dialog.getByRole('button', { name: 'Continue to the demo' }).tap();
    await expect(dialog).toBeHidden();
    await expect(page.locator('#demo-frame')).toBeFocused();
    await expect(page.frameLocator('#demo-frame').getByText('Coffee on Thursday?')).toBeVisible();
    await context.close();
  });
}

test('unknown platform falls back to Linux with every tab available', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'userAgent', { get: () => 'Unrecognized browser' });
    Object.defineProperty(navigator, 'platform', { get: () => '' });
    Object.defineProperty(navigator, 'userAgentData', { get: () => undefined });
  });
  await page.goto('/');
  await expect(page.locator('#suggested-install')).toHaveText('Get Shep');
  await expect(page.locator('#linux')).toBeVisible();
  for (const platform of ['macos', 'windows', 'android', 'ios']) await expect(page.locator(`#${platform}`)).toBeHidden();
});

test('platform tabs switch with the mouse, arrow keys and links', async ({ page }, testInfo) => {
  await page.goto('/#download');
  await page.getByRole('tab', { name: 'Windows' }).click();
  await expect(page.locator('#windows')).toBeVisible();
  await expect(page.locator('#command-windows')).toHaveText(commands.windows);
  await expect(page.locator('#windows').getByRole('link', { name: 'Full installation guide' })).toHaveAttribute('href', 'https://sam-ruff.github.io/shep.so/installation/#windows');
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('tab', { name: 'Android' })).toBeFocused();
  await expect(page.locator('#android')).toBeVisible();
  await expect(page.locator('#android').getByRole('link', { name: /live demo/ })).toHaveAttribute('href', 'demo/');
  await page.keyboard.press('Home');
  await expect(page.getByRole('tab', { name: 'Linux' })).toBeFocused();
  await page.keyboard.press('ArrowLeft');
  await expect(page.locator('#ios')).toBeVisible();
  await page.goto('/#macos');
  await expect(page.locator('#macos')).toBeVisible();
  await expect(page.locator('#command-macos')).toHaveText(commands.macos);
  await expect(page.locator('#linux')).toBeHidden();
  await screenshot(page, testInfo, 'macos-tab', false);
});

test('install command copy success and denied-clipboard recovery', async ({ page, context, browserName }) => {
  test.skip(browserName !== 'chromium', 'Clipboard permissions differ by engine; recovery is also covered in the storage-denial flow.');
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto('/#linux');
  await page.locator('#linux').getByRole('button', { name: 'Copy' }).click();
  await expect(page.locator('#linux').getByRole('status')).toHaveText('Copied. Paste it into a terminal to install Shep.');
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(commands.linux);
  await page.getByRole('tab', { name: 'Windows' }).click();
  await page.locator('#windows').getByRole('button', { name: 'Copy' }).click();
  await expect(page.locator('#windows').getByRole('status')).toHaveText('Copied. Paste it into PowerShell to install Shep.');
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(commands.windows);
  await page.addInitScript(() => Object.defineProperty(navigator, 'clipboard', { get: () => ({ writeText: () => Promise.reject(new Error('Fixture denial')) }) }));
  await page.reload();
  await page.getByRole('tab', { name: 'Linux' }).click();
  await page.locator('#linux').getByRole('button', { name: 'Copy' }).click();
  await expect(page.locator('#linux').getByRole('status')).toHaveText('Copy is unavailable. Select the command above and copy it.');
});

test('the live demo runs in the hero on page load and follows the appearance', async ({ page }, testInfo) => {
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  const frame = page.locator('#demo-frame');
  await expect(frame).toBeVisible();
  await expect(frame).toHaveAttribute('src', 'demo/app/');
  const demo = page.frameLocator('#demo-frame');
  await demo.getByText('Coffee on Thursday?').click();
  await expect(demo.getByRole('heading', { name: 'Coffee on Thursday?' })).toBeVisible();
  await expect(demo.getByRole('link', { name: 'Get Shep' })).toHaveCount(0);
  await screenshot(page, testInfo, 'demo-running', false);
  await page.getByLabel('Appearance').selectOption('dark');
  await expect(frame).toHaveAttribute('src', 'demo/app/?appearance=dark');
  await expect(demo.locator('html')).toHaveAttribute('data-theme', 'dark');
  for (const link of await page.locator('.demo-open').all()) await expect(link).toHaveAttribute('href', 'demo/?appearance=dark');
  await page.reload();
  await expect(frame).toHaveAttribute('src', 'demo/app/?appearance=dark');
  expect(errors).toEqual([]);
});

test('loading the demo does not take focus or scroll the page', async ({ page }) => {
  await page.goto('/');
  await expect(page.frameLocator('#demo-frame').getByText('Coffee on Thursday?')).toBeVisible();
  expect(await page.evaluate(() => [window.scrollY, document.activeElement === document.body])).toEqual([0, true]);
});

test('full screen opens a new tab with the demo filling the window', async ({ page, context, request }, testInfo) => {
  expect((await request.get('/demo/app/assets/', { maxRedirects: 0 })).status()).toBe(404);
  await page.goto('/');
  await page.getByLabel('Appearance').selectOption('dark');
  const [tab] = await Promise.all([context.waitForEvent('page'), page.getByRole('link', { name: /^Full screen/ }).click()]);
  await expect(tab).toHaveURL(`${origin}/demo/?appearance=dark`);
  await expect(tab).toHaveTitle('Shep live demo');
  const frame = tab.locator('#demo-frame');
  await expect(frame).toHaveAttribute('src', 'app/?appearance=dark');
  await expect(tab.locator('#demo-notice')).not.toHaveAttribute('open');
  await expect(tab.locator('#demo-notice')).toBeHidden();
  const size = await tab.evaluate(() => {
    const box = document.querySelector('#demo-frame').getBoundingClientRect();
    return [box.x, box.y, box.width === innerWidth, box.height === innerHeight];
  });
  expect(size).toEqual([0, 0, true, true]);
  const demo = tab.frameLocator('#demo-frame');
  await demo.getByText('A little room for good ideas').click();
  await expect(demo.getByRole('heading', { name: 'A little room for good ideas' })).toBeVisible();
  await expect(demo.locator('html')).toHaveAttribute('data-theme', 'dark');
  await screenshot(tab, testInfo, 'demo-full-screen', false);
  expect(await tab.evaluate(() => localStorage.getItem('shep.preferences.v1'))).toBeNull();
  for (const name of ['Open the demo full screen', 'Live demo']) {
    const [other] = await Promise.all([context.waitForEvent('page'), page.getByRole('link', { name, exact: true }).click()]);
    await expect(other).toHaveURL(`${origin}/demo/?appearance=dark`);
    await other.close();
  }
  await expect(page).toHaveURL(`${origin}/`);
});

test('keyboard skip link, install navigation and appearance control', async ({ page }, testInfo) => {
  await page.goto('/');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('link', { name: 'Skip to content' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('main')).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.locator('#suggested-install')).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(`${origin}/#download`);
  await page.locator('#appearance').focus();
  await expect(page.locator('#appearance')).toBeFocused();
  await screenshot(page, testInfo, 'keyboard-focus', false);
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
  await page.getByRole('tab', { name: 'Linux' }).click();
  await page.locator('#linux').getByRole('button', { name: 'Copy' }).click();
  await expect(page.locator('#linux').getByRole('status')).toContainText('Select the command');
  expect(errors).toEqual([]);
});

for (const [name, width, height, theme] of [
  ['wide-light', 1920, 1080, 'light'],
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
    await page.locator('#suggested-install').click();
    await expect(page.locator('#download-title')).toBeInViewport();
    await expect(page.locator('.install-panel:visible')).toHaveCount(1);
    const results = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
    expect(results.violations).toEqual([]);
    await page.getByRole('link', { name: 'Shep home' }).first().click();
    await expect(page.getByRole('link', { name: 'Open the demo full screen' })).toBeVisible();
    await expect(page.getByRole('link', { name: /^Full screen/ })).toBeVisible();
    await expect(page.frameLocator('#demo-frame').locator('html')).toHaveAttribute('data-theme', theme);
    // Desktop browsers keep the demo at every width, including phone-sized windows.
    await expect(page.locator('#demo-notice')).toBeHidden();
    expect(await page.locator('#demo-frame').evaluate(frame => frame.inert)).toBe(false);
    await checkNoOverflow(page);
    await screenshot(page, testInfo, name);
    await screenshot(page, testInfo, `${name}-hero`, false);
  });
}

test('installation and content remain usable with JavaScript disabled', async ({ browser }) => {
  const context = await browser.newContext({ javaScriptEnabled: false, viewport: { width: 390, height: 844 } });
  const page = await context.newPage();
  await page.goto(origin);
  await page.getByRole('link', { name: 'Get Shep', exact: true }).first().click();
  await expect(page).toHaveURL(/#download$/);
  for (const platform of ['linux', 'macos', 'windows', 'android', 'ios']) await expect(page.locator(`#${platform}`)).toBeVisible();
  await expect(page.locator('#command-linux')).toHaveText(commands.linux);
  await expect(page.getByRole('button', { name: 'Copy' })).toHaveCount(0);
  await expect(page.getByRole('tab')).toHaveCount(0);
  await expect(page.getByRole('link', { name: 'Open the demo full screen' })).toHaveAttribute('href', 'demo/');
  await expect(page.locator('#demo-frame')).toHaveAttribute('src', 'demo/app/');
  await context.close();
});
