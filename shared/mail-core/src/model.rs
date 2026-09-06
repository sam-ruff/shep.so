use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_MESSAGE_BYTES: usize = 25 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Protocol {
    #[default]
    Imap,
    Pop3,
}
impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Imap => "IMAP",
            Self::Pop3 => "POP3",
        })
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConnectionSecurity {
    #[default]
    Tls,
    StartTls,
}
impl fmt::Display for ConnectionSecurity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Tls => "SSL/TLS",
            Self::StartTls => "STARTTLS",
        })
    }
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum IncomingAuth {
    #[default]
    Password,
    Plain,
}
impl fmt::Display for IncomingAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Password => "Normal password",
            Self::Plain => "SASL PLAIN",
        })
    }
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SmtpAuth {
    #[default]
    Automatic,
    Plain,
    Login,
    None,
}
impl fmt::Display for SmtpAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Automatic => "Automatic",
            Self::Plain => "PLAIN",
            Self::Login => "LOGIN",
            Self::None => "No authentication",
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionTarget {
    Incoming,
    Smtp,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SentCopyPolicy {
    #[default]
    Automatic,
    ServerManaged,
    LocalOnly,
}
impl std::fmt::Display for SentCopyPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Automatic => "Save a copy on the mail server",
            Self::ServerManaged => "My server saves Sent automatically",
            Self::LocalOnly => "Keep Sent copies only on this device",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub email: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    #[serde(default)]
    pub incoming_security: ConnectionSecurity,
    #[serde(default)]
    pub incoming_auth: IncomingAuth,
    #[serde(default)]
    pub smtp_security: Option<ConnectionSecurity>,
    #[serde(default)]
    pub smtp_auth: SmtpAuth,
    #[serde(default)]
    pub smtp_username: String,
    #[serde(default)]
    pub smtp_separate_password: bool,
    #[serde(default)]
    pub sent_copy: SentCopyPolicy,
    #[serde(default)]
    pub sent_folder: String,
}
impl Account {
    pub fn smtp_security(&self) -> ConnectionSecurity {
        self.smtp_security.unwrap_or(if self.smtp_port == 465 {
            ConnectionSecurity::Tls
        } else {
            ConnectionSecurity::StartTls
        })
    }
    pub fn smtp_username(&self) -> &str {
        if self.smtp_username.is_empty() {
            &self.username
        } else {
            &self.smtp_username
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.sent_folder.len() <= 1024 && !self.sent_folder.contains(['\r', '\n', '\0']),
            "Choose a valid Sent folder."
        );
        anyhow::ensure!(!self.name.trim().is_empty(), "Give this account a name.");
        self.email
            .parse::<lettre::message::Mailbox>()
            .map_err(|_| anyhow::anyhow!("Enter a valid email address."))?;
        anyhow::ensure!(
            !self.host.trim().is_empty() && !self.host.contains(['/', '\r', '\n', ' ']),
            "Enter a mail server hostname, without https://."
        );
        anyhow::ensure!(
            self.port > 0 && self.smtp_port > 0,
            "Ports must be between 1 and 65535."
        );
        anyhow::ensure!(
            !self.username.is_empty() && !self.username.contains(['\r', '\n']),
            "Enter a valid username."
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mail {
    pub id: String,
    pub account_id: String,
    pub remote_id: String,
    pub folder: String,
    pub sender: String,
    pub recipient: String,
    pub subject: String,
    pub preview: String,
    pub timestamp: i64,
    pub unread: bool,
    pub starred: bool,
    pub attachment_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMail {
    pub summary: Mail,
    #[serde(with = "base64_bytes")]
    pub raw: Vec<u8>,
    pub text: String,
}

mod base64_bytes {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteImage {
    pub url: String,
    pub alt: String,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImagePolicy {
    #[default]
    BlockAll,
    Contacts,
    AllowAll,
}
impl fmt::Display for ImagePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BlockAll => "Block all",
            Self::Contacts => "Contacts",
            Self::AllowAll => "Allow all",
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplyDisplay {
    #[default]
    Collapsed,
    Expanded,
    LatestOnly,
}
impl fmt::Display for ReplyDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Collapsed => "Collapse earlier replies",
            Self::Expanded => "Expand earlier replies",
            Self::LatestOnly => "Latest text only",
        })
    }
}

#[derive(Debug, Clone)]
pub struct MailDetail {
    pub summary: Mail,
    pub body: String,
    pub body_truncated: bool,
    pub remote_images: Vec<RemoteImage>,
    pub latest_body: String,
    pub replies: Vec<crate::replies::ReplySection>,
    pub attachments: std::sync::Arc<Vec<Attachment>>,
    pub reply: crate::compose::ReplyHeaders,
}

#[derive(Debug)]
pub enum MailSyncItem {
    Message(StoredMail),
    Flags(Vec<(String, bool, bool)>),
    Reconcile {
        account: String,
        folder: String,
        live_ids: std::collections::HashSet<String>,
    },
    SkippedLarge,
    Folders(String, Vec<String>),
    SentFolder(String, Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Draft {
    pub id: String,
    pub account_id: String,
    pub to: String,
    pub subject: String,
    pub body: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub bcc: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
    // File bytes and associations have separate storage; saving text cannot
    // undo a file import/removal that finished while the user was typing.
    #[serde(default, skip_serializing)]
    pub attachments: Vec<DraftAttachment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftAttachment {
    pub id: String,
    pub name: String,
    pub media_type: String,
    pub size: usize,
}

pub fn parse_mail(
    account_id: &str,
    remote_id: &str,
    folder: &str,
    raw: Vec<u8>,
    unread: bool,
    starred: bool,
) -> anyhow::Result<StoredMail> {
    use mailparse::MailHeaderMap;
    let parsed = mailparse::parse_mail(&raw)?;
    let subject = parsed
        .headers
        .get_first_value("Subject")
        .unwrap_or_else(|| "(No subject)".into());
    let sender = parsed.headers.get_first_value("From").unwrap_or_default();
    let recipient = parsed.headers.get_first_value("To").unwrap_or_default();
    let timestamp = parsed
        .headers
        .get_first_value("Date")
        .and_then(|d| mailparse::dateparse(&d).ok())
        .unwrap_or_else(|| Utc::now().timestamp());
    let (text, attachments) = content(&parsed);
    let preview = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(180)
        .collect();
    let summary = Mail {
        id: format!("{account_id}:{folder}:{remote_id}"),
        account_id: account_id.into(),
        remote_id: remote_id.into(),
        folder: folder.into(),
        sender,
        recipient,
        subject,
        preview,
        timestamp,
        unread,
        starred,
        attachment_count: attachments.len(),
    };
    Ok(StoredMail { summary, raw, text })
}

pub fn content(parsed: &mailparse::ParsedMail<'_>) -> (String, Vec<Attachment>) {
    let mut plain = Vec::new();
    let mut html = Vec::new();
    let mut attachments = Vec::new();
    fn walk(
        p: &mailparse::ParsedMail<'_>,
        plain: &mut Vec<String>,
        html: &mut Vec<String>,
        attachments: &mut Vec<Attachment>,
    ) {
        let disp = p.get_content_disposition();
        if disp.disposition == mailparse::DispositionType::Attachment
            || disp.params.contains_key("filename")
        {
            attachments.push(Attachment {
                name: disp
                    .params
                    .get("filename")
                    .cloned()
                    .unwrap_or_else(|| "attachment.bin".into()),
                bytes: p.get_body_raw().unwrap_or_default(),
            });
        } else if p.subparts.is_empty() {
            if p.ctype.mimetype == "text/plain" {
                plain.push(p.get_body().unwrap_or_default());
            } else if p.ctype.mimetype == "text/html" {
                html.push(p.get_body().unwrap_or_default());
            }
        } else {
            for part in &p.subparts {
                walk(part, plain, html, attachments);
            }
        }
    }
    walk(parsed, &mut plain, &mut html, &mut attachments);
    let text = if plain.is_empty() {
        html.into_iter()
            .map(|h| html_to_text(&h))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        plain.join("\n")
    };
    (text, attachments)
}

fn html_to_text(html: &str) -> String {
    fn walk(element: scraper::ElementRef<'_>, output: &mut String, quoted: bool) {
        let name = element.value().name();
        if matches!(name, "style" | "script" | "head" | "title" | "noscript") {
            return;
        }
        let block = matches!(
            name,
            "p" | "div" | "br" | "tr" | "li" | "blockquote" | "h1" | "h2" | "h3"
        );
        if block && !output.ends_with('\n') {
            output.push('\n');
        }
        let quoted = quoted || name == "blockquote";
        for node in element.children() {
            if let Some(child) = scraper::ElementRef::wrap(node) {
                walk(child, output, quoted);
            } else if let Some(text) = node.value().as_text() {
                let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !normalized.is_empty() {
                    if output.ends_with('\n') && quoted {
                        output.push_str("> ");
                    } else if !output.ends_with(['\n', ' ']) && !output.is_empty() {
                        output.push(' ');
                    }
                    output.push_str(&normalized);
                }
            }
        }
        if block && !output.ends_with('\n') {
            output.push('\n');
        }
    }
    let document = scraper::Html::parse_document(html);
    let mut output = String::new();
    walk(document.root_element(), &mut output, false);
    output.trim().into()
}
