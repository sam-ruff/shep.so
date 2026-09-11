# Build from the repository root: docker build -f website/Dockerfile .
# scripts/build.mjs copies logos and screenshots from the repository root, so
# the context is the root and only the paths it reads are copied in.
FROM node:24-alpine AS build

WORKDIR /app/website

COPY website/package.json website/package-lock.json ./

RUN npm ci

COPY website/public ./public
COPY website/scripts ./scripts
COPY assets/logo-light.webp assets/logo-dark.webp /app/assets/
COPY docs/images/mail-light.webp docs/images/calendar-dark.webp /app/docs/images/

RUN npm run build

# Chainguard nginx pinned by digest, matching the other images on the estate.
FROM cgr.dev/chainguard/nginx@sha256:fe77a7f4da2ed98d5629bab3f4f2a0cc40ab18b341cb46a62f05acce7a760c82

COPY --from=build /app/website/dist /usr/share/nginx/html
COPY website/nginx.conf /etc/nginx/conf.d/default.conf

EXPOSE 8080
