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

test('Docker context allows only the promotional and demo build inputs', () => {
  const patterns = read('website/Dockerfile.dockerignore').trim().split('\n');
  assert.deepEqual(patterns, [
    '**', '!website/', '!website/Dockerfile', '!website/package.json',
    '!website/package-lock.json', '!website/nginx.conf', '!website/public/',
    '!website/public/**', '!website/scripts/', '!website/scripts/build.mjs',
    '!assets/', '!assets/shepherd-light.svg', '!docs/',
    '!docs/images/', '!docs/images/mail-light.webp', '!docs/images/calendar-dark.webp',
    '!Cargo.toml', '!Cargo.lock', '!build.rs', '!src/', '!src/**', '!benches/', '!benches/**',
    '!vendor/', '!vendor/**', '!shared/', '!shared/**', 'shared/**/target/',
    '!scripts/', '!scripts/clients/', '!scripts/clients/build_mail_content.mjs',
    '!web/', '!web/package.json', '!web/package-lock.json', '!web/index.html',
    '!web/preview.html', '!web/print.html', '!web/tsconfig.json', '!web/vite.config.ts',
    '!web/public/', '!web/public/**', '!web/src/', '!web/src/**', 'web/src/wasm/',
  ]);
  const docker = read('website/Dockerfile');
  for (const stage of [/FROM rust:[\d.]+-slim-bookworm@sha256:[a-f0-9]{64} AS wasm/, /FROM node:24\.17\.0-alpine@sha256:[a-f0-9]{64} AS demo/, /FROM scratch AS demo-files/, /FROM node:24\.17\.0-alpine@sha256:[a-f0-9]{64} AS build/]) {
    assert.match(docker, stage);
  }
  assert.match(docker, /--version 0\.2\.128 wasm-bindgen-cli/);
  assert.match(docker, /node scripts\/clients\/build_mail_content\.mjs/);
  assert.match(docker, /COPY --from=demo \/app\/web\/dist-preview \/app\/web\/dist-preview/);
});

test('CI builds the demo through the image stage and serves it with a trailing slash', () => {
  const workflow = read('.github/workflows/website.yml');
  assert.match(workflow, /--target demo-files --output type=local,dest=web\/dist-preview/);
  for (const path of ['web/**', 'shared/**', 'Cargo.lock']) assert.ok(workflow.includes(`- "${path}"`), path);
  const nginx = read('website/nginx.conf');
  assert.match(nginx, /location ~ \^\/demo\(\/app\)\?\$ \{\n\s+return 301 \$uri\/;/);
  assert.match(nginx, /absolute_redirect off;/);
});
