import assert from 'node:assert/strict';
import test from 'node:test';
import { planRelease } from './release_plan.mjs';

const source = 'a'.repeat(40);

test('analysis cannot execute prepare, commit or publication plugins', async () => {
  let calls = 0;
  const plan = await planRelease(source, async (options) => {
    calls++;
    assert.equal(options.dryRun, true);
    assert.deepEqual(options.branches, ['main']);
    assert.deepEqual(options.plugins, [
      '@semantic-release/commit-analyzer', '@semantic-release/release-notes-generator',
    ]);
    return { nextRelease: { version: '1.2.3' } };
  });
  assert.equal(calls, 1);
  assert.equal(plan.version, '1.2.3');
  assert.equal(plan.source, source);
  assert.equal(plan.scope, 'desktop');
  assert.deepEqual(plan.targets, ['x86_64-unknown-linux-gnu', 'x86_64-pc-windows-msvc']);
});

test('no release remains explicit; invalid source cannot call the analyser', async () => {
  assert.equal((await planRelease(source, async () => false)).version, null);
  await assert.rejects(planRelease('not-a-sha', async () => {
    assert.fail('Invalid source must never reach release tooling');
  }));
});

test('analysis errors and invalid versions do not create a release plan', async () => {
  await assert.rejects(planRelease(source, async () => { throw new Error('No authenticated repository'); }));
  await assert.rejects(planRelease(source, async () => ({ nextRelease: { version: '1.2.3\npublish=yes' } })));
});
