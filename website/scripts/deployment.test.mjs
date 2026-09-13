import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8');

test('publisher retains exact source identity without application release tags', () => {
  const workflow = read('.github/workflows/website.yml');
  assert.match(workflow, /sha=sha-\$\{GITHUB_SHA\}/);
  assert.match(workflow, /--build-arg GIT_SHA="\$GITHUB_SHA"/);
  assert.match(workflow, /--build-arg VERSION="\$\{\{ steps\.tag\.outputs\.sha \}\}"/);
  assert.doesNotMatch(workflow, /GITHUB_SHA::|:latest|semantic-release|gh release/);
  const docker = read('website/Dockerfile');
  assert.match(docker, /org\.opencontainers\.image\.source="https:\/\/github\.com\/sam-ruff\/shep\.so"/);
  assert.match(docker, /org\.opencontainers\.image\.revision="\$\{GIT_SHA\}"/);
  assert.match(docker, /org\.opencontainers\.image\.version="\$\{VERSION\}"/);
});

test('pull requests validate but both registry steps require main publication', () => {
  const workflow = read('.github/workflows/website.yml');
  assert.match(workflow, /^  pull_request:/m);
  assert.match(workflow, /PUBLISH:.*github\.ref == 'refs\/heads\/main'.*github\.event_name != 'pull_request'/);
  assert.match(workflow, /name: Log in to the registry\n\s+if: env\.PUBLISH == 'true'/);
  assert.match(workflow, /name: Push image\n\s+if: env\.PUBLISH == 'true'/);
  assert.match(workflow, /runs-on: \[self-hosted, sophie\]/);
  assert.match(workflow, /persist-credentials: false/);
  for (const action of workflow.matchAll(/uses: ([^\n]+)/g)) {
    assert.match(action[1], /@[a-f0-9]{40}$/);
  }
});

test('unversioned assets revalidate and unknown paths remain unavailable', () => {
  const nginx = read('website/nginx.conf');
  assert.match(nginx, /try_files \$uri \$uri\/ =404/);
  assert.match(nginx, /Cache-Control "no-cache, must-revalidate"/);
  assert.match(nginx, /Cache-Control "no-store, must-revalidate"/);
  assert.doesNotMatch(nginx, /immutable|expires 1y/);
});

test('Docker context allows only the promotional build inputs', () => {
  const patterns = read('website/Dockerfile.dockerignore').trim().split('\n');
  assert.deepEqual(patterns, [
    '**', '!website/', '!website/Dockerfile', '!website/package.json',
    '!website/package-lock.json', '!website/nginx.conf', '!website/public/',
    '!website/public/**', '!website/scripts/', '!website/scripts/build.mjs',
    '!assets/', '!assets/logo-light.webp', '!assets/logo-dark.webp', '!docs/',
    '!docs/images/', '!docs/images/mail-light.webp', '!docs/images/calendar-dark.webp',
  ]);
  assert.match(read('website/Dockerfile'), /FROM node:24\.17\.0-alpine@sha256:[a-f0-9]{64} AS build/);
});
