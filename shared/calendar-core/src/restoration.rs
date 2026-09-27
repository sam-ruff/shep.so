//! Conditional restoration of an acknowledged organiser-owned cancellation.

use crate::{Event, ProviderFailure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveStatus {
    Confirmed,
    Tentative,
}

impl LiveStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Tentative => "tentative",
        }
    }
}

/// Persist this exact plan before claiming the provider mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DeletePlanFields", into = "DeletePlanFields")]
pub struct DeletePlan {
    before: Event,
    status: LiveStatus,
    uid: String,
    organiser: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeletePlanFields {
    before: Event,
    status: LiveStatus,
    uid: String,
    organiser: String,
}

impl DeletePlan {
    pub(crate) fn new(
        before: Event,
        status: LiveStatus,
        uid: String,
        organiser: String,
    ) -> Result<Self, ProviderFailure> {
        if !before.is_bounded()
            || before.end <= before.start
            || !before.etag.as_deref().is_some_and(strong_etag)
            || before.remote_url.as_deref() != Some(before.id.as_str())
            || uid.is_empty()
            || uid.len() > 1024
            || organiser.is_empty()
            || organiser.len() > 1024
        {
            return Err(ProviderFailure::rejected("Invalid calendar deletion plan."));
        }
        Ok(Self {
            before,
            status,
            uid,
            organiser,
        })
    }

    pub fn before(&self) -> &Event {
        &self.before
    }
    pub fn status(&self) -> LiveStatus {
        self.status
    }
    pub fn uid(&self) -> &str {
        &self.uid
    }
    pub fn organiser(&self) -> &str {
        &self.organiser
    }
}

impl TryFrom<DeletePlanFields> for DeletePlan {
    type Error = ProviderFailure;
    fn try_from(value: DeletePlanFields) -> Result<Self, Self::Error> {
        Self::new(value.before, value.status, value.uid, value.organiser)
    }
}

impl From<DeletePlan> for DeletePlanFields {
    fn from(value: DeletePlan) -> Self {
        Self {
            before: value.before,
            status: value.status,
            uid: value.uid,
            organiser: value.organiser,
        }
    }
}

/// Only a successful cancellation response may create this receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DeleteReceiptFields", into = "DeleteReceiptFields")]
pub struct DeleteReceipt {
    plan: DeletePlan,
    cancelled_etag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteReceiptFields {
    plan: DeletePlan,
    cancelled_etag: String,
}

impl DeleteReceipt {
    pub(crate) fn acknowledged(
        plan: DeletePlan,
        cancelled_etag: String,
    ) -> Result<Self, ProviderFailure> {
        if !strong_etag(&cancelled_etag)
            || plan.before.etag.as_deref() == Some(cancelled_etag.as_str())
        {
            return Err(ProviderFailure::uncertain(
                "Google returned no new cancellation version.",
            ));
        }
        Ok(Self {
            plan,
            cancelled_etag,
        })
    }
    pub fn plan(&self) -> &DeletePlan {
        &self.plan
    }
    pub fn cancelled_etag(&self) -> &str {
        &self.cancelled_etag
    }
    pub fn restore_request(&self) -> RestoreRequest {
        RestoreRequest {
            receipt: self.clone(),
        }
    }
}

impl TryFrom<DeleteReceiptFields> for DeleteReceipt {
    type Error = ProviderFailure;
    fn try_from(value: DeleteReceiptFields) -> Result<Self, Self::Error> {
        Self::acknowledged(value.plan, value.cancelled_etag)
    }
}

impl From<DeleteReceipt> for DeleteReceiptFields {
    fn from(value: DeleteReceipt) -> Self {
        Self {
            plan: value.plan,
            cancelled_etag: value.cancelled_etag,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreRequest {
    receipt: DeleteReceipt,
}

impl RestoreRequest {
    pub fn receipt(&self) -> &DeleteReceipt {
        &self.receipt
    }
}

/// An observation never authorises a replacement mutation receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inspection {
    Missing,
    Live { event: Event, status: LiveStatus },
    Cancelled { etag: String },
}

pub(crate) fn strong_etag(value: &str) -> bool {
    (3..=4096).contains(&value.len())
        && value.starts_with('"')
        && value.ends_with('"')
        && value[1..value.len() - 1]
            .bytes()
            .all(|byte| byte >= 0x21 && byte != b'"' && byte != 0x7f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditional_versions_are_single_strong_header_values() {
        for value in ["\"version\"", "\"123\""] {
            assert!(strong_etag(value));
        }
        for value in [
            "",
            "*",
            "version",
            "\"\"",
            "W/\"version\"",
            "\"a\", \"b\"",
            "\"a\r\nb\"",
        ] {
            assert!(!strong_etag(value));
        }
        assert!(!strong_etag(&format!("\"{}\"", "v".repeat(4096))));
    }
}
