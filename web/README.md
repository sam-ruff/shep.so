# Shep browser

Separate desktop-style browser client for Shep. Restrained light/dark appearance, folder/list/reader panes, saved resizing and configurable shortcuts. **Development beta:** account verification, mail refresh/actions and reserved SMTP use the Rust gateway. Mail/raw messages, drafts and send receipts stay in a Google-identity-scoped IndexedDB profile; passwords stay in tab memory. Full parity remains [open](../docs/CLIENT_PARITY.md).

```sh
npm ci
npm run dev
```

Open `http://127.0.0.1:5180/preview.html` for fictional mail. `/` requires a verified beta session before opening the production client. `npm run build` excludes the preview entry; `npm run build:preview` creates `dist-preview/` for review only.

`npm test` checks ordering/rollback. `npm run e2e` drives actual browser controls. The Rust gateway serves `/app/` after Google login and an exact administrator allowlist check. Calendar, backups, attachment downloads, remembered credentials and complete recovery UI remain open. Mail/passwords have no persistent server store; VPS deployment configuration remains pending. Current code stays in review worktrees; deployment awaits the supplied VPS/OAuth configuration.
