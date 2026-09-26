import { access, cp, mkdir, rename, rm } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const root = new URL('../../', import.meta.url);
const output = new URL('website/dist/', root);
const demo = process.env.SHEP_DEMO_DIR ? pathToFileURL(`${resolve(process.env.SHEP_DEMO_DIR)}/`) : new URL('web/dist-preview/', root);

try {
  await access(new URL('preview.html', demo));
} catch {
  console.error(`The browser demo is missing from ${demo.pathname}. Run "npm run build:demo" first.`);
  process.exit(1);
}

await rm(output, { recursive: true, force: true });
await mkdir(new URL('assets/', output), { recursive: true });
await cp(new URL('website/public/', root), output, { recursive: true });
for (const [source, name] of [
  ['docs/images/mail-light.webp', 'mail-light.webp'],
  ['docs/images/calendar-dark.webp', 'calendar-dark.webp'],
  ['assets/shepherd-light.svg', 'shepherd.svg'],
]) {
  await cp(new URL(source, root), new URL(`assets/${name}`, output));
}
await cp(demo, new URL('demo/', output), { recursive: true });
await rename(new URL('demo/preview.html', output), new URL('demo/index.html', output));
console.log('Built website/dist with the launcher logo, native screenshots and the browser demo.');
