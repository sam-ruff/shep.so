import { cp, mkdir, rm } from 'node:fs/promises';

const root = new URL('../../', import.meta.url);
const output = new URL('website/dist/', root);
await rm(output, { recursive: true, force: true });
await mkdir(new URL('assets/', output), { recursive: true });
await cp(new URL('website/public/', root), output, { recursive: true });
for (const [source, name] of [
  ['docs/images/mail-light.webp', 'mail-light.webp'],
  ['docs/images/calendar-dark.webp', 'calendar-dark.webp'],
  ['assets/logo-light.webp', 'logo-light.webp'],
  ['assets/logo-dark.webp', 'logo-dark.webp'],
]) {
  await cp(new URL(source, root), new URL(`assets/${name}`, output));
}
console.log('Built website/dist with the approved logo and native demo screenshots.');
