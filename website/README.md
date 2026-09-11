# Shep website

Static promotion for Shep, with browser-aware installation, the approved logo and native Linux screenshots containing fictional mail/events. `web/` owns the browser mail client; `flutter/` owns the mobile app. The planned browser entry is an invite-only Google beta at `/beta`, initially restricted to the owner.

```sh
cd website
npm ci
npm run build
npm run preview
```

Open `http://127.0.0.1:4178`. The self-contained output is `website/dist/`. Build copies the existing WebP screenshots and logos; no duplicate source assets are maintained. There are no remote fonts, analytics or runtime dependencies. Only appearance is stored locally.

For tests, install Python 3 with Pillow and the Playwright browsers:

```sh
npx playwright install chromium firefox webkit
npm test
npm run test:all-browsers
```

The Playwright flows use real browser controls, as in Walkie Textie's browser automation. They cover platform suggestions (including desktop-mode iPad and ChromeOS), all-platform navigation, keyboard focus, appearance persistence, blocked storage/clipboard, no-JavaScript navigation, link targets, image loading, six viewport/theme combinations and axe WCAG checks. Reports, traces and WebP screenshots go to ignored `artifacts/website/`. Automated accessibility checks complement visual/keyboard review; they do not establish full screen-reader coverage. Browser emulation does not establish native mobile verification.

The site ships as a container: a node build stage produces `dist/` and a Chainguard nginx stage serves it on port 8080. The build script reads logos and screenshots from the repository root, so build from there with `docker build -f website/Dockerfile -t shep-website .` and run it with `docker run --rm -p 8080:8080 shep-website`. Unknown paths return 404 rather than the index, so `/beta` can be served by a separate container behind the same host. The `Website` workflow runs the Chromium Playwright suite and, on pushes to `main`, pushes the image to `registry.tail2d6fbe.ts.net/shep/website` tagged `latest` and `sha-<short sha>` using the `ZOT_USERNAME` and `ZOT_PASSWORD` repository secrets.

Distribution cards deliberately show unpublished Google Play/App Store and undeployed private-beta status. The beta is not open for login yet. Replace those labels with verified release/store destinations when available. The Linux guide currently installs from source. Keep availability and product limits aligned with the root README and completion audit.

`backend/` implements the Google access gate and SMTP/IMAP/POP3 transport without persistent server mail or password storage. Full client/provider parity remains in TODO.md. The VPS SSH target and exact allowlisted Google identity are pending; this promotional site does not implement or verify authentication.

The site is committed on the combined `feat/mobile-web-clients` review branch. Deployment, DNS, store publication and release activation remain pending. Serve this output at `/` and integrate the independently built gated beta at `/beta` when verified. Keep documentation publishing at its existing destination and quality/release workflows disabled.
