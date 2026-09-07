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
