use crate::{Error, Result, text};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

macro_rules! enumeration {
    ($name:ident { $($variant:ident),+ }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { $($variant),+ }
    };
}
enumeration!(Protocol { Imap, Pop3 });
enumeration!(Security { Tls, StartTls });
enumeration!(IncomingAuth { Password, Plain });
enumeration!(SmtpAuth {
    Automatic,
    Plain,
    Login,
    None
});
enumeration!(SentCopy {
    Automatic,
    ServerManaged,
    LocalOnly
});

/// Complete, explicit connection identity. No implicit TLS/auth defaults, local
/// credential slot or OAuth grant. A name is an independent AccountName change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub id: Uuid,
    pub email: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub incoming_security: Security,
    pub incoming_auth: IncomingAuth,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: String,
    pub smtp_security: Security,
    pub smtp_auth: SmtpAuth,
    pub smtp_separate_password: bool,
    pub sent_copy: SentCopy,
    pub sent_folder: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
impl Connection {
    pub fn validate(&self) -> Result<()> {
        crate::json::portable_map(&self.extra)?;
        if self.id.is_nil()
            || !text(&self.email, 320, false)
            || !self.email.contains('@')
            || !host(&self.host)
            || !host(&self.smtp_host)
            || self.port == 0
            || self.smtp_port == 0
            || !text(&self.username, 1024, false)
            || !text(&self.smtp_username, 1024, false)
            || !text(&self.sent_folder, 1024, true)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
fn host(value: &str) -> bool {
    text(value, 253, false)
        && !value.chars().any(char::is_whitespace)
        && !value.contains(['/', '\\', '@', '?', '#'])
}
