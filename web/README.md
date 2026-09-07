# Shep browser

Separate desktop-style browser client for Shep. Restrained light/dark appearance, folder/list/reader panes, saved resizing and configurable shortcuts. **Development beta:** account verification, mail refresh/actions and reserved SMTP use the Rust gateway. Mail/raw messages, drafts and send receipts stay in a Google-identity-scoped IndexedDB profile; passwords stay in tab memory. Full parity remains [open](../docs/CLIENT_PARITY.md).

```sh
npm ci
npm run dev
```

Open `http://127.0.0.1:5180/preview.html` for fictional mail. `/` requires a verified beta session before opening the production client. `npm run build` excludes the preview entry; `npm run build:preview` creates `dist-preview/` for review only.

`npm test` checks ordering/rollback. `npm run e2e` drives actual browser controls. The Rust gateway serves `/app/` after Google login and an exact administrator allowlist check. Sent-copy policies, provider lookup and reviewed copy recovery are available in Preferences and Outbox. Cached attachment downloads, Sent identity handover/grouping and formatted HTML reading have synthetic control evidence. Calendar, backups, remembered credentials and complete recovery UI remain open. Mail/passwords have no persistent server store; VPS deployment configuration remains pending. Current code stays in review worktrees; deployment awaits the supplied VPS/OAuth configuration.

Formatted messages use shared Rust/WASM preparation in a cancellable worker, retaining tables, authored styles and bounded inline images. Selection, Find, quoted history and a plain-text choice remain available. Sender scripts and automatic remote images are blocked; remote-image exceptions/loading are still pending. Build the gateway and web app from the same revision so their display-runtime CSP hashes match.
