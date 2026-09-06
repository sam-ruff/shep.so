# Shared Rust mail contracts

Mail models, MIME/reply construction and IMAP/POP3/SMTP implementations used by the root desktop app and `backend/`. Root modules keep compatibility exports; there is one protocol implementation. This crate has no iced, database, filesystem cache or keychain dependency.

Run `cargo test -p shep-mail-core` from the monorepo root. Root `cargo test --all-features` includes this workspace member. Protocol fixtures cover FETCH lists, UIDVALIDITY, flags, MOVE acknowledgments, POP3, Sent and SMTP failure/retry contracts. Pinned gateway connections additionally verify TLS/STARTTLS and hostname rejection with a scoped synthetic CA. Fixture TLS keys never enter production APIs.

The native provider resolves its configured hostname normally. The hosted gateway uses administrator-supplied IP pins while validating TLS against the original hostname; browsers cannot use it as an unrestricted proxy. Flutter binding and browser cache/provider integration remain tracked in the client parity matrix.
