# Build from the repository root: docker build -f website/Dockerfile .
# scripts/build.mjs copies logos and screenshots from the repository root, so
# the context is the root and only the paths it reads are copied in.
FROM node:24.17.0-alpine@sha256:156b55f92e98ccd5ef49578a8cea0df4679826564bad1c9d4ef04462b9f0ded6 AS build

WORKDIR /app/website

COPY website/package.json website/package-lock.json ./

RUN npm ci

COPY website/public ./public
COPY website/scripts ./scripts
COPY assets/logo-light.webp assets/logo-dark.webp /app/assets/
COPY docs/images/mail-light.webp docs/images/calendar-dark.webp /app/docs/images/

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
