//! Draft composition and RFC mail construction, independent of the UI/transport.
use crate::model::*;
use anyhow::Context;
use lettre::message::{Mailbox, Mailboxes, MultiPart, SinglePart, header::ContentType};
use mailparse::MailHeaderMap;
use std::collections::HashSet;

mod forwarding;
pub use forwarding::ForwardQuote;

pub const MAX_ATTACHMENT_BYTES: usize = 18 * 1024 * 1024;
pub const MAX_ATTACHMENTS: usize = 32;

#[derive(Debug, Clone, Default)]
pub struct ReplyHeaders {
    pub reply_to: Vec<Mailbox>,
    pub to: Vec<Mailbox>,
    pub cc: Vec<Mailbox>,
    pub message_id: Option<String>,
    pub references: Vec<String>,
}
impl ReplyHeaders {
    pub fn parse(mail: &mailparse::ParsedMail<'_>) -> Self {
        let addresses = |header: &str| {
            mail.headers
                .get_all_values(header)
                .iter()
                .flat_map(|value| {
                    value
                        .parse::<Mailboxes>()
                        .map(|list| list.into_iter().collect::<Vec<_>>())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        };
        let reply_to = addresses("Reply-To");
        Self {
            reply_to: if reply_to.is_empty() {
                addresses("From")
            } else {
                reply_to
            },
            to: addresses("To"),
            cc: addresses("Cc"),
            message_id: mail
                .headers
                .get_first_value("Message-ID")
                .and_then(|value| message_ids(&value).into_iter().next()),
            references: mail
                .headers
                .get_first_value("References")
                .map(|value| message_ids(&value))
                .unwrap_or_else(|| {
                    mail.headers
                        .get_first_value("In-Reply-To")
                        .map(|value| message_ids(&value))
                        .unwrap_or_default()
                }),
        }
    }

    pub fn draft<Html>(&self, mail: &MailDetail<Html>, accounts: &[Account], all: bool) -> Draft {
        let own: HashSet<_> = accounts
            .iter()
            .filter_map(|account| account.email.parse::<Mailbox>().ok())
            .map(|mailbox| mailbox.email.to_string().to_ascii_lowercase())
            .collect();
        let mut used = HashSet::new();
        let mut unique = |mailbox: &Mailbox| {
            let key = mailbox.email.to_string().to_ascii_lowercase();
            !own.contains(&key) && used.insert(key)
        };
        let mut to: Vec<_> = self
            .reply_to
            .iter()
            .filter(|m| unique(m))
            .cloned()
            .collect();
        // Replying to a message we sent should address its recipients.
        if to.is_empty() {
            to.extend(self.to.iter().filter(|m| unique(m)).cloned());
        }
        if all {
            to.extend(self.to.iter().filter(|m| unique(m)).cloned());
        }
        let cc: Vec<_> = if all {
            self.cc.iter().filter(|m| unique(m)).cloned().collect()
        } else {
            Vec::new()
        };
        let mut references = self.references.clone();
        if let Some(id) = &self.message_id
            && !references.contains(id)
        {
            references.push(id.clone());
        }
        if references.len() > 100 {
            references.drain(..references.len() - 100);
        }
        let subject = if mail
            .summary
            .subject
            .get(..3)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("re:"))
        {
            mail.summary.subject.clone()
        } else {
            format!("Re: {}", mail.summary.subject)
        };
        Draft {
            id: uuid::Uuid::new_v4().to_string(),
            account_id: mail.summary.account_id.clone(),
            to: to
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            cc: cc
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            subject,
            in_reply_to: self.message_id.clone(),
            references,
            body: format!(
                "\n\nOn {}, {} wrote:\n> {}",
                chrono::DateTime::from_timestamp(mail.summary.timestamp, 0)
                    .unwrap_or_default()
                    .format("%d %b %Y"),
                mail.summary.sender,
                mail.body
                    .lines()
                    .map(|line| line.to_owned())
                    .collect::<Vec<_>>()
                    .join("\n> ")
            ),
            ..Default::default()
        }
    }
}

fn valid_message_id(id: &str) -> bool {
    id.starts_with('<')
        && id.ends_with('>')
        && id.len() <= 998
        && id.len() > 3
        && id[1..id.len() - 1].contains('@')
        && id[1..id.len() - 1]
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'<' | b'>'))
}
#[doc(hidden)]
pub fn message_ids(value: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for part in value.split('<').skip(1) {
        if let Some(end) = part.find('>') {
            let id = format!("<{}>", &part[..end]);
            if valid_message_id(&id) && !ids.contains(&id) {
                ids.push(id);
            }
            if ids.len() == 100 {
                break;
            }
        }
    }
    ids
}

pub fn recipients(value: &str, label: &str) -> anyhow::Result<Vec<Mailbox>> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    anyhow::ensure!(
        value.len() <= 16 * 1024 && !value.contains(['\r', '\n', '\0']),
        "Check the email addresses in {label}."
    );
    value
        .parse::<Mailboxes>()
        .map(|mailboxes| mailboxes.into_iter().collect())
        .map_err(|_| {
            anyhow::anyhow!("Check the email addresses in {label}. Use commas between recipients.")
        })
}

pub struct FilePart {
    pub attachment: DraftAttachment,
    pub bytes: Vec<u8>,
}

pub fn build(
    account: &Account,
    draft: &Draft,
    files: Vec<FilePart>,
) -> anyhow::Result<lettre::Message> {
    build_with_message_id(
        account,
        draft,
        files,
        &format!("<{}@shep.local>", uuid::Uuid::new_v4()),
    )
}

/// A durable client submission can retain one exact wire identity across retry
/// review. This does not itself authorize retrying an ambiguous SMTP delivery.
pub fn build_with_message_id(
    account: &Account,
    draft: &Draft,
    files: Vec<FilePart>,
    message_id: &str,
) -> anyhow::Result<lettre::Message> {
    anyhow::ensure!(
        valid_message_id(message_id),
        "Invalid outgoing message identity."
    );
    let from = account
        .email
        .parse::<Mailbox>()
        .context("Check this account's sender address.")?;
    let to = recipients(&draft.to, "To")?;
    let cc = recipients(&draft.cc, "Cc")?;
    let bcc = recipients(&draft.bcc, "Bcc")?;
    let mut addresses = Vec::new();
    let mut seen = HashSet::new();
    for mailbox in to.iter().chain(&cc).chain(&bcc) {
        if seen.insert(mailbox.email.to_string().to_ascii_lowercase()) {
            addresses.push(mailbox.email.clone());
        }
    }
    anyhow::ensure!(
        !addresses.is_empty(),
        "Enter at least one recipient in To, Cc or Bcc."
    );
    anyhow::ensure!(
        addresses.len() <= 100,
        "Send to at most 100 recipients at a time."
    );
    anyhow::ensure!(
        !draft.subject.contains(['\r', '\n', '\0']),
        "The subject must be a single line."
    );
    anyhow::ensure!(
        draft.body.len() <= MAX_MESSAGE_BYTES,
        "This message exceeds the current 25 MiB sending limit."
    );
    let envelope = lettre::address::Envelope::new(Some(from.email.clone()), addresses)?;
    let mut builder = lettre::Message::builder()
        .from(from)
        .subject(&draft.subject)
        .message_id(Some(message_id.to_owned()))
        .envelope(envelope);
    for mailbox in to {
        builder = builder.to(mailbox);
    }
    for mailbox in cc {
        builder = builder.cc(mailbox);
    }
    // Explicit envelope carries Bcc recipients. Never add Bcc to wire headers.
    if let Some(id) = &draft.in_reply_to {
        anyhow::ensure!(
            valid_message_id(id),
            "The reply contains an invalid original message ID."
        );
        builder = builder.in_reply_to(id.clone());
    }
    anyhow::ensure!(
        draft.references.len() <= 100 && draft.references.iter().all(|id| valid_message_id(id)),
        "The reply contains invalid message references."
    );
    if !draft.references.is_empty() {
        builder = builder.references(draft.references.join(" "));
    }
    anyhow::ensure!(
        files.len() == draft.attachments.len() && files.len() <= MAX_ATTACHMENTS,
        "An attachment is missing. Reopen the draft and check its files."
    );
    let html = draft
        .forward
        .as_ref()
        .and_then(|quote| quote.render(&draft.body));
    let mut attachments = Vec::new();
    let mut inline = Vec::new();
    let mut total = 0usize;
    let mut content_ids = HashSet::new();
    for (file, expected) in files.into_iter().zip(&draft.attachments) {
        anyhow::ensure!(
            &file.attachment == expected && file.bytes.len() == expected.size,
            "An attachment changed. Reopen the draft and check its files."
        );
        total = total
            .checked_add(file.bytes.len())
            .context("Attachment size overflow")?;
        anyhow::ensure!(
            total <= MAX_ATTACHMENT_BYTES,
            "Attachments must total 18 MiB or less."
        );
        let kind = ContentType::parse(&file.attachment.media_type)
            .context("An attachment has an invalid media type.")?;
        if let Some(cid) = &expected.content_id {
            anyhow::ensure!(
                !cid.is_empty()
                    && cid.len() <= 998
                    && !cid.contains(['\r', '\n', '\0', '<', '>'])
                    && content_ids.insert(cid.clone()),
                "An inline image has an invalid or repeated Content-ID."
            );
        }
        if let Some(cid) = expected.content_id.as_ref().filter(|_| html.is_some()) {
            inline.push(
                lettre::message::Attachment::new_inline_with_name(
                    cid.clone(),
                    file.attachment.name,
                )
                .body(file.bytes, kind),
            );
        } else {
            attachments.push(
                lettre::message::Attachment::new(file.attachment.name).body(file.bytes, kind),
            );
        }
    }
    let plain = SinglePart::plain(draft.body.clone());
    let message = if html.is_none() && attachments.is_empty() {
        builder.singlepart(plain)?
    } else {
        let mut multipart = if let Some(html) = html {
            let mut related = MultiPart::related().singlepart(SinglePart::html(html));
            for part in inline {
                related = related.singlepart(part);
            }
            MultiPart::mixed().multipart(
                MultiPart::alternative()
                    .singlepart(plain)
                    .multipart(related),
            )
        } else {
            MultiPart::mixed().singlepart(plain)
        };
        for part in attachments {
            multipart = multipart.singlepart(part);
        }
        builder.multipart(multipart)?
    };
    anyhow::ensure!(
        message.formatted().len() <= MAX_MESSAGE_BYTES,
        "This message exceeds the current 25 MiB sending limit. Remove attachments or shorten the message."
    );
    Ok(message)
}

/// Parsed envelope data for a cached browser reply. Addresses retain Rust's
/// validated mailbox formatting; clients never infer recipients from display text.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReplyAddress {
    pub email: String,
    pub text: String,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReplyEnvelope {
    pub reply_to: Vec<ReplyAddress>,
    pub to: Vec<ReplyAddress>,
    pub cc: Vec<ReplyAddress>,
    pub message_id: Option<String>,
    pub references: Vec<String>,
}
impl ReplyHeaders {
    pub fn envelope(&self) -> ReplyEnvelope {
        let addresses = |values: &[Mailbox]| {
            values
                .iter()
                .map(|m| ReplyAddress {
                    email: m.email.to_string(),
                    text: m.to_string(),
                })
                .collect()
        };
        ReplyEnvelope {
            reply_to: addresses(&self.reply_to),
            to: addresses(&self.to),
            cc: addresses(&self.cc),
            message_id: self.message_id.clone(),
            references: self.references.clone(),
        }
    }
}
pub fn reply_from_raw(
    summary: Mail,
    raw: &[u8],
    accounts: &[Account],
    all: bool,
) -> anyhow::Result<Draft> {
    let parsed = crate::mime::parse(raw)?;
    let reply = ReplyHeaders::parse(&parsed);
    let (body, _) = content(&parsed)?;
    let detail = MailDetail::<()> {
        html: None,
        summary,
        body: body.clone(),
        body_truncated: false,
        remote_images: Vec::new(),
        latest_body: body,
        replies: Vec::new(),
        attachments: Default::default(),
        reply: reply.clone(),
    };
    Ok(reply.draft(&detail, accounts, all))
}
