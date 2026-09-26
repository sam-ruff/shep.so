# Build from the repository root: docker build -f website/Dockerfile .
# The site copies logos and screenshots from the repository root and embeds the
# browser client's fictional-mail demo, so the context is the root and only the
# paths those builds read are copied in.

# Shared MIME and profile crates compiled to WebAssembly for the demo's workers.
FROM rust:1.96.0-slim-bookworm@sha256:4732ca96fd086cb9be682050c3f0176288eebaac2b80aa2bcefccfaf198e1950 AS wasm

# Node only runs the web client's own build script, so crates and features stay in one place.
RUN apt-get update \
 && apt-get install -y --no-install-recommends nodejs \
 && rm -rf /var/lib/apt/lists/* \
 && rustup target add wasm32-unknown-unknown \
 && cargo install --locked --root /opt/wasm-tools --version 0.2.128 wasm-bindgen-cli

WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY benches ./benches
COPY vendor ./vendor
COPY shared ./shared
COPY scripts/clients/build_mail_content.mjs ./scripts/clients/

RUN SHEP_WASM_BINDGEN=/opt/wasm-tools/bin/wasm-bindgen node scripts/clients/build_mail_content.mjs \
 && mv web/src/wasm /wasm

FROM node:24.17.0-alpine@sha256:156b55f92e98ccd5ef49578a8cea0df4679826564bad1c9d4ef04462b9f0ded6 AS demo

WORKDIR /app/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --ignore-scripts
COPY shared/*.json /app/shared/
COPY web/index.html web/preview.html web/print.html web/tsconfig.json web/vite.config.ts ./
COPY web/public ./public
COPY web/src ./src
COPY --from=wasm /wasm ./src/wasm
# The package script would rebuild the WebAssembly, which the stage above owns.
RUN npx tsc --noEmit && npx vite build --mode preview

# `docker build --target demo-files --output type=local,dest=web/dist-preview .`
# exports the demo for local or CI website builds without a Rust toolchain.
FROM scratch AS demo-files
COPY --from=demo /app/web/dist-preview /

FROM node:24.17.0-alpine@sha256:156b55f92e98ccd5ef49578a8cea0df4679826564bad1c9d4ef04462b9f0ded6 AS build

WORKDIR /app/website

COPY website/package.json website/package-lock.json ./

RUN npm ci

COPY website/public ./public
COPY website/scripts ./scripts
COPY assets/shepherd-light.svg /app/assets/
COPY docs/images/mail-light.webp docs/images/calendar-dark.webp /app/docs/images/
COPY --from=demo /app/web/dist-preview /app/web/dist-preview

RUN npm run build

# Chainguard nginx pinned by digest, matching the other images on the estate.
FROM cgr.dev/chainguard/nginx@sha256:7b3bbe97d6ba4be0a00719e08abd483b7d18ae46b65f5c20b83a681691d9e691

ARG GIT_SHA
ARG VERSION
LABEL org.opencontainers.image.source="https://github.com/sam-ruff/shep.so" \
      org.opencontainers.image.revision="${GIT_SHA}" \
      org.opencontainers.image.version="${VERSION}"

COPY --from=build /app/website/dist /usr/share/nginx/html
COPY website/nginx.conf /etc/nginx/conf.d/default.conf

EXPOSE 8080
