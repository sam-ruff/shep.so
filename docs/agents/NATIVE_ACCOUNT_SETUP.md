# Native account setup

Account setup saves a nonsecret request before waiting for a provider. The previous account and its credentials remain active until incoming and SMTP checks succeed. Preferences shows saved connection activity and offers configuration recovery without retaining passwords in the form or SQLite.

`store/account_setup.rs` owns request identity, captured settings, progress and device credential references. `credentials/account_setup.rs` writes an independent incoming/SMTP pair and verifies the writes. `engine/account_setup.rs` uses the existing network owner for probes and rechecks the exact request before committing account settings and credential references together. Newer requests, removal and changed settings invalidate old activation. Replaying an acknowledged activation also verifies its current ownership.

The local FIFO orders admission and close interruption. A separate write barrier drains credential staging and activation; it never spans provider probes or unrelated mail reads. Close records unresolved requests as interrupted even when their network commands are behind occupied provider capacity. Restart exposes configuration recovery and requires credential entry. It does not promote staged credentials or automatically replay interrupted setup.

Provider credential reads resolve the checked binding while holding the account operation lock. Logical backup credentials remain account identities, never reusable device slot references. Restoring a missing checked key requires reconnection. Database imports archive local requests and remove their bindings. Profile password imports activate the checked pair and exact synced revisions in one SQLite transaction. Cleanup uses raw slot references, checks durable ownership and remains retryable after keychain failure.

## Connection edits

Incoming mailbox identity follows the existing exact protocol, host, port, username, security and authentication contract. Case changes are conservatively distinct; there is no new hostname or username normalisation. An account with cached mail cannot change this identity through setup. Reconnect with its existing incoming settings, or add another account. Moving cached identities to another incoming connection requires a reviewed migration.

Display name, email alias and Sent role are not incoming mailbox identity. SMTP changes can be checked and activated independently, but existing queued outgoing submissions retain their original configuration and require review when that configuration changes. An SMTP account with no authentication requires no separate password slot, including when an old separate-password flag remains set.

## Remaining boundaries

The outgoing owner still reports a definite failure when credential lookup fails before SMTP dispatch. A shared Waiting/reconnect recovery presentation is a separate adapter change; this work does not change unknown SMTP delivery semantics. Incoming mailbox identity migration remains unavailable. Browser and Flutter use their own credential owners and require explicit parity review rather than importing native slot references.
