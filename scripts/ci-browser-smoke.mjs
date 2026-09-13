import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, openSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { once } from 'node:events';

const evidence = resolve(process.argv[2] ?? 'artifacts/logs/browser-smoke');
mkdirSync(evidence, { recursive: true });
const profile = mkdtempSync(join(tmpdir(), 'shep-ci-browser-'));
const log = openSync(join(evidence, 'browser.log'), 'w');
let display;
let browser;
let deadline;
let nextId = 0;
const pending = new Map();

function request(method, params = {}, sessionId) {
  const id = ++nextId;
  return new Promise((resolveResult, reject) => {
    pending.set(id, { resolve: resolveResult, reject });
    browser.stdio[3].write(JSON.stringify({ id, method, params, sessionId }) + '\0');
  });
}

async function smoke() {
  display = spawn('Xvfb', ['-displayfd', '3', '-screen', '0', '1440x920x24', '-nolisten', 'tcp'],
    { detached: true, stdio: ['ignore', log, log, 'pipe'] });
  const [number] = await once(display.stdio[3], 'data');
  assert.match(number.toString(), /^\d+\n$/);
  browser = spawn('google-chrome', [
    `--user-data-dir=${profile}`, '--ozone-platform=x11', '--remote-debugging-pipe',
    '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
    '--disable-component-update', '--disable-sync', 'about:blank',
  ], { detached: true, env: { ...process.env, DISPLAY: `:${number.toString().trim()}` },
    stdio: ['ignore', log, log, 'pipe', 'pipe'] });
  let buffer = '';
  browser.stdio[4].setEncoding('utf8');
  browser.stdio[4].on('data', chunk => {
    buffer += chunk;
    let boundary;
    while ((boundary = buffer.indexOf('\0')) !== -1) {
      const message = JSON.parse(buffer.slice(0, boundary));
      buffer = buffer.slice(boundary + 1);
      const operation = pending.get(message.id);
      if (!operation) continue;
      pending.delete(message.id);
      if (message.error) operation.reject(new Error(JSON.stringify(message.error)));
      else operation.resolve(message.result);
    }
  });
  browser.once('exit', code => {
    for (const operation of pending.values()) operation.reject(new Error(`Chrome exited: ${code}`));
    pending.clear();
  });
  const version = await request('Browser.getVersion');
  const { targetId } = await request('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await request('Target.attachToTarget', { targetId, flatten: true });
  await request('Page.enable', {}, sessionId);
  await request('Page.navigate', { url: 'data:text/html,<title>Shep browser smoke</title><h1>Shep sandbox preflight</h1>' }, sessionId);
  const text = await request('Runtime.evaluate', {
    expression: `new Promise(resolve => { const inspect = () => document.readyState === 'complete' && document.querySelector('h1') ? resolve(document.body.innerText) : setTimeout(inspect, 20); inspect(); })`,
    awaitPromise: true, returnByValue: true,
  }, sessionId);
  assert.equal(text.result.value, 'Shep sandbox preflight');
  const screenshot = await request('Page.captureScreenshot', { format: 'png' }, sessionId);
  writeFileSync(join(evidence, 'rendered.png'), Buffer.from(screenshot.data, 'base64'));
  const pdf = await request('Page.printToPDF', {}, sessionId);
  writeFileSync(join(evidence, 'rendered.pdf'), Buffer.from(pdf.data, 'base64'));
  const pdfText = execFileSync('pdftotext', [join(evidence, 'rendered.pdf'), '-'], { encoding: 'utf8', timeout: 5000 });
  assert.ok(pdfText.includes('Shep sandbox preflight'));
  await request('Page.navigate', { url: 'chrome://sandbox' }, sessionId);
  const sandbox = await request('Runtime.evaluate', {
    expression: `new Promise(resolve => { const inspect = () => document.body?.innerText.includes('Seccomp-BPF') ? resolve(document.body.innerText) : setTimeout(inspect, 20); inspect(); })`,
    awaitPromise: true, returnByValue: true,
  }, sessionId);
  assert.match(sandbox.result.value, /Layer 1 Sandbox\s+Namespace/i);
  assert.match(sandbox.result.value, /PID namespaces\s+Yes/i);
  assert.match(sandbox.result.value, /Network namespaces\s+Yes/i);
  assert.match(sandbox.result.value, /Seccomp-BPF sandbox\s+Yes/i);
  writeFileSync(join(evidence, 'receipt.json'), JSON.stringify({ version, text: text.result.value, sandbox: sandbox.result.value, pdfText }, null, 2));
  await request('Browser.close');
  console.log('Headed Chrome rendered and printed the fixture with namespace and seccomp sandboxes enabled.');
}

try {
  await Promise.race([smoke(), new Promise((_, reject) => {
    deadline = setTimeout(() => reject(new Error('Browser preflight exceeded 30 seconds')), 30000);
  })]);
} finally {
  clearTimeout(deadline);
  for (const child of [browser, display]) {
    if (!child?.pid) continue;
    try { process.kill(-child.pid, 'SIGKILL'); } catch (error) {
      if (error.code !== 'ESRCH') throw error;
    }
  }
  rmSync(profile, { recursive: true, force: true });
}
