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

Distribution cards deliberately show unpublished Google Play/App Store and undeployed private-beta status. The beta is not open for login yet. Replace those labels with verified release/store destinations when available. The Linux guide currently installs from source. Keep availability and product limits aligned with the root README and completion audit.

The browser's earlier static-only plan is superseded: `backend/` implements the Google access gate; SMTP/IMAP/POP3 transport on the email VPS is still being ported, without persisting mail or passwords there. The VPS SSH target and exact allowlisted Google identity are pending; this promotional site does not implement or verify authentication.

This work remains uncommitted in its worktree. Deployment, DNS, store publication and release activation are deferred. Serve this output at `/` and integrate the independently built gated beta at `/beta` when verified. Keep documentation publishing at its existing destination and quality/release workflows disabled.
