# Shep beta gateway

Rust service for the hosted browser beta. Google sign-in and an administrator allowlist gate **both `/app/` assets and `/api/`**, initially for the owner's explicitly supplied email only. Empty allowlists deny everyone. Sessions stay in memory and expire after eight hours; a restart signs everyone out. Additional allowed emails can be configured later.

The gateway includes IMAP/POP3 sync, connection probes, IMAP flags/MOVE and reserved SMTP submissions through `shared/mail-core`. **Provider Sent-copy recovery and full client parity are unfinished.** The browser adapter verifies accounts, caches mail/drafts locally and durably saves the exact outgoing MIME/envelope before SMTP. Mail stays disabled (501) until administrator endpoint pins are configured. There is no server mailbox/password database or user keychain access.

`SHEP_MAIL_ENDPOINTS` lists exact hostname, displayed port, protocol and pinned TCP address. TLS still verifies the original hostname, including STARTTLS. No arbitrary browser-supplied destination is permitted. Sync is streamed with bounded buffers; SMTP requires authentication and a server-issued reservation. Preparation returns the exact MIME/envelope and local Sent metadata; only a digest of the non-secret account settings and wire content remains in the receipt. Send accepts matching prepared bytes. Repeated IDs return the existing receipt; unknown IDs after a restart cannot silently start another send. An atomic cancel closes an unused reservation without releasing an active SMTP operation. Browser Outbox offers explicit delivery review and keeps confirmed receipts locally after server expiry. Credentials/MIME exist only for the active request/operation; provider Sent lookup/copy recovery remains open.

```sh
cargo test --manifest-path backend/Cargo.toml
npm --prefix web run build
cargo test --manifest-path backend/Cargo.toml real_browser_beta_gate -- --ignored
cargo build --release --manifest-path backend/Cargo.toml
```

`deploy/beta.env.example` documents server configuration. Set up a Google **Web application** OAuth client with `https://shep.so/auth/callback`, then supply the owner's exact Google email in `SHEP_BETA_EMAILS`. `SHEP_BETA_SUBJECTS` can additionally pin Google's stable subject ID. Do not infer either identity from Git metadata or use a Flutter/native OAuth client here.

The gateway binds only to loopback behind HTTPS. Review `deploy/shep-beta.service` and `deploy/Caddyfile.example` alongside the existing mail VPS configuration. Public promo assets and protected web assets use separate directories. OAuth credentials belong in a root-readable environment file outside the repository. Never enable request-body/header/callback-URL logging, proxy disk buffering, mail caches or crash dumps for future credential-bearing transport.

The implementation uses fixed Google endpoints, bounded HTTP responses, RS256 validation with issuer/audience/expiry/nonce checks, email verification, one-use state/cookie/PKCE, Secure/HttpOnly/SameSite cookies, CSRF/origin checks and no-store responses. Tests use a fake verifier and a clearly synthetic RSA test key, never real Google credentials. An additional Playwright flow exercises the actual Rust gate and production browser over local HTTPS, including denied users and UI logout. It substitutes Google identity exchange and the mail transport in a Rust test binary; shared-core TLS tests separately exercise the actual wire protocols. Those tests are not live authorization or VPS verification.

VPS SSH target, owner identity and OAuth configuration are pending. No server was installed or deployed. Quality/release CI remains disabled; current work stays in worktrees.
