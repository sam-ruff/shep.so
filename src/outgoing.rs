//! Durable identities for delivery and Sent-copy recovery. No credentials live here.
use crate::model::*;
use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryState {
    Submitting,
    Uncertain,
    Rejected,
    Accepted,
    Complete,
    Released,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SentState {
    Pending,
    Appending,
    Uncertain,
    Saved,
    LocalOnly,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingInfo {
    pub attempt: String,
    pub draft_id: String,
    pub draft_revision: u64,
    pub account_id: String,
    #[serde(default)]
    pub from: String,
    pub subject: String,
    pub to: String,
    pub created: i64,
    pub message_id: String,
    pub delivery: DeliveryState,
    pub sent: SentState,
    pub folder: Option<String>,
    pub error: Option<String>,
}
impl OutgoingInfo {
    pub fn local_remote_id(&self) -> String {
        format!("local-sent-{}", self.attempt)
    }
    pub fn local_id(&self) -> String {
        format!("{}:Sent:{}", self.account_id, self.local_remote_id())
    }
    pub fn needs_delivery_review(&self) -> bool {
        matches!(
            self.delivery,
            DeliveryState::Submitting | DeliveryState::Uncertain
        )
    }
}
#[derive(Debug, Clone, Default)]
pub struct OutgoingPage {
    pub revision: u64,
    pub offset: usize,
    pub total: usize,
    pub rows: Vec<OutgoingInfo>,
}
pub const OUTGOING_PAGE_SIZE: usize = 20;

#[derive(Serialize, Deserialize)]
pub struct EnvelopeData {
    pub from: String,
    pub to: Vec<String>,
}
impl EnvelopeData {
    pub fn envelope(&self) -> anyhow::Result<lettre::address::Envelope> {
        Ok(lettre::address::Envelope::new(
            Some(self.from.parse()?),
            self.to
                .iter()
                .map(|s| s.parse())
                .collect::<Result<_, _>>()?,
        )?)
    }
}
pub struct Submission {
    pub info: OutgoingInfo,
    pub account: Account,
    pub envelope: EnvelopeData,
    pub raw: Vec<u8>,
}
impl Submission {
    pub fn new(account: Account, draft: &Draft, message: lettre::Message) -> anyhow::Result<Self> {
        use mailparse::MailHeaderMap;
        let raw = message.formatted();
        anyhow::ensure!(
            raw.len() <= MAX_MESSAGE_BYTES,
            "This message exceeds the sending size limit."
        );
        let (headers, _) = mailparse::parse_headers(&raw)?;
        let message_id = headers
            .get_first_value("Message-ID")
            .context("The outgoing message has no identity.")?;
        anyhow::ensure!(
            crate::compose::message_ids(&message_id) == [message_id.clone()],
            "Invalid outgoing message identity."
        );
        let envelope = EnvelopeData {
            from: message
                .envelope()
                .from()
                .context("The outgoing sender is missing.")?
                .to_string(),
            to: message
                .envelope()
                .to()
                .iter()
                .map(ToString::to_string)
                .collect(),
        };
        Ok(Self {
            info: OutgoingInfo {
                attempt: uuid::Uuid::new_v4().to_string(),
                draft_id: draft.id.clone(),
                draft_revision: draft.revision,
                account_id: account.id.clone(),
                from: account.email.clone(),
                subject: draft.subject.clone(),
                to: if !draft.to.is_empty() {
                    draft.to.clone()
                } else if !draft.cc.is_empty() {
                    draft.cc.clone()
                } else {
                    format!("{} hidden recipients", envelope.to.len())
                },
                created: chrono::Utc::now().timestamp(),
                message_id,
                delivery: DeliveryState::Submitting,
                sent: SentState::Pending,
                folder: None,
                error: None,
            },
            account,
            envelope,
            raw,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    CheckSent,
    RetryCopy,
    MarkSent,
    ReturnDraft,
    KeepLocal,
}
