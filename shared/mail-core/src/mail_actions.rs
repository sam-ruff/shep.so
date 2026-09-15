use crate::model::Mail;

/// Only the fields explicitly changed by the user are written to the server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Flags {
    pub unread: Option<bool>,
    pub starred: Option<bool>,
}

impl Flags {
    pub fn between(before: &Mail, after: &Mail) -> Self {
        Self {
            unread: (before.unread != after.unread).then_some(after.unread),
            starred: (before.starred != after.starred).then_some(after.starred),
        }
    }
    pub fn apply(self, mail: &mut Mail) {
        if let Some(value) = self.unread {
            mail.unread = value;
        }
        if let Some(value) = self.starred {
            mail.starred = value;
        }
    }
    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

/// The server definitely did not apply a move and will not on a plain retry:
/// a tagged NO/BAD without any COPYUID, a refused destination creation, or an
/// operation the server does not support. Nothing partial happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveRefused(pub String);
impl std::fmt::Display for MoveRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for MoveRefused {}

/// How a failed server move may be handled locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveFailure {
    /// Proven not applied; the move can complete locally and be retried later.
    Refused,
    /// Unknown or transient: connection loss, timeout, authentication, a
    /// partial MOVE (RFC 6851 section 3.3) or an unconfirmed folder creation.
    Uncertain,
}

/// Pure classification over provider errors. Only typed refusals count; every
/// message-only error stays uncertain so a retry can never repeat a move that
/// may already have been applied.
pub fn classify_move_failure(error: &anyhow::Error) -> MoveFailure {
    let refused = error.chain().any(|cause| {
        cause.downcast_ref::<MoveRefused>().is_some()
            || cause
                .downcast_ref::<crate::providers::mail::receipts::UploadRejected>()
                .is_some()
            || cause
                .downcast_ref::<crate::folder_actions::creation::CreationRejected>()
                .is_some()
    });
    if refused {
        MoveFailure::Refused
    } else {
        MoveFailure::Uncertain
    }
}

/// Compact identity proof retained with a move receipt, never a UI body buffer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Fingerprint {
    pub bytes: u64,
    pub sha256: [u8; 32],
    pub message_id: Option<String>,
}
impl Fingerprint {
    pub fn of(raw: &[u8]) -> Self {
        use mailparse::MailHeaderMap;
        use sha2::{Digest, Sha256};
        let message_id = mailparse::parse_headers(&raw[..raw.len().min(64 * 1024)])
            .ok()
            .and_then(|(headers, _)| {
                let ids = headers.get_all_values("Message-ID");
                if ids.len() != 1 {
                    return None;
                }
                let ids = crate::compose::message_ids(&ids[0]);
                (ids.len() == 1).then(|| ids[0].clone())
            });
        Self {
            bytes: raw.len() as u64,
            sha256: Sha256::digest(raw).into(),
            message_id,
        }
    }
    pub fn matches(&self, raw: &[u8]) -> bool {
        use sha2::{Digest, Sha256};
        self.bytes == raw.len() as u64 && self.sha256 == <[u8; 32]>::from(Sha256::digest(raw))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MoveReceipt {
    pub account: String,
    pub folder: String,
    pub current: Option<Mail>,
    pub fingerprint: Option<Fingerprint>,
    #[serde(default)]
    pub connections: Vec<(String, String)>,
    /// Links acknowledged results to protected cache/restart recovery.
    #[serde(default)]
    pub recovery: Option<String>,
    /// The server refused the move, so it was applied on this device only and
    /// the server move is retried later. Display metadata; the journal stage
    /// is the durable truth.
    #[serde(default)]
    pub local_only: bool,
}
impl MoveReceipt {
    pub fn server(
        source: &Mail,
        account: &str,
        folder: &str,
        remote_id: Option<String>,
        fingerprint: Fingerprint,
    ) -> Self {
        let current = remote_id.map(|remote_id| {
            let mut mail = source.clone();
            mail.id = format!("{account}:{folder}:{remote_id}");
            mail.account_id = account.into();
            mail.folder = folder.into();
            mail.remote_id = remote_id;
            mail
        });
        Self {
            account: account.into(),
            folder: folder.into(),
            current,
            fingerprint: Some(fingerprint),
            connections: Vec::new(),
            recovery: None,
            local_only: false,
        }
    }
    pub fn local(source: &Mail, folder: &str) -> Self {
        let mut mail = source.clone();
        mail.folder = folder.into();
        Self {
            account: mail.account_id.clone(),
            folder: folder.into(),
            current: Some(mail),
            fingerprint: None,
            connections: Vec::new(),
            recovery: None,
            local_only: false,
        }
    }
}

/// Only incoming-server identity is frozen; renaming an account or changing its
/// SMTP/preferences must not prevent reversing an existing mail move.
pub fn connection_key(account: &crate::model::Account) -> String {
    use sha2::{Digest, Sha256};
    let data = serde_json::to_vec(&(
        account.protocol,
        &account.host,
        account.port,
        &account.username,
        account.incoming_security,
        account.incoming_auth,
    ))
    .expect("Incoming account identity contains only serializable fields");
    format!("{:x}", Sha256::digest(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_typed_refusals_classify_as_refused_even_through_context() {
        let refused: Vec<anyhow::Error> = vec![
            MoveRefused("NO read-only".into()).into(),
            anyhow::Error::from(MoveRefused("BAD".into())).context("Connecting failed"),
            crate::providers::mail::receipts::UploadRejected.into(),
            crate::folder_actions::creation::CreationRejected("denied".into()).into(),
        ];
        for error in &refused {
            assert_eq!(
                classify_move_failure(error),
                MoveFailure::Refused,
                "{error:#}"
            );
        }
        let uncertain: Vec<anyhow::Error> = vec![
            anyhow::anyhow!("The server acknowledgment timed out."),
            anyhow::anyhow!("connection reset").context("Login failed"),
            anyhow::anyhow!("The server did not confirm this operation."),
            anyhow::anyhow!("The connection ended before folder creation was confirmed."),
        ];
        for error in &uncertain {
            assert_eq!(
                classify_move_failure(error),
                MoveFailure::Uncertain,
                "{error:#}"
            );
        }
    }

    #[test]
    fn receipts_deserialise_without_the_local_only_marker() {
        let receipt: MoveReceipt = serde_json::from_str(
            r#"{"account":"work","folder":"Archive","current":null,"fingerprint":null}"#,
        )
        .expect("older receipt");
        assert!(!receipt.local_only);
        assert!(receipt.recovery.is_none());
    }
}
