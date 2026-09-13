#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import { appendFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export async function planRelease(source, release) {
  if (!/^[0-9a-f]{40}$/.test(source)) throw new Error('Expected a complete source revision');
  const result = await release({
    dryRun: true,
    branches: ['main'],
    plugins: ['@semantic-release/commit-analyzer', '@semantic-release/release-notes-generator'],
  });
  const version = result ? result.nextRelease.version : null;
  if (version !== null && !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error('Release analysis returned an invalid version');
  }
  return { schema: 1, source, scope: 'desktop', version,
    targets: ['x86_64-unknown-linux-gnu', 'x86_64-pc-windows-msvc'] };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const source = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
  const { default: release } = await import('semantic-release');
  const plan = await planRelease(source, release);
  const path = resolve('artifacts/release-plan.json');
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(plan, null, 2)}\n`);
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `version=${plan.version ?? ''}\n`);
  console.log(plan.version ? `Desktop release planned: ${plan.version}` : 'No desktop release required');
}
