use super::{
    enrollment::{Options, Snapshot},
    setup::Discovery,
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum Request {
    Change {
        request: u64,
        changes: super::enrollment::Changes,
    },
    Status(u64),
    Page {
        request: u64,
        review: Arc<Discovery>,
        after: Option<String>,
    },
    JoinReview {
        request: u64,
        review: Arc<Discovery>,
        cursor: String,
    },
    JoinAccept {
        request: u64,
        review: Arc<super::join::Review>,
    },
    Options {
        request: u64,
        revision: u64,
        options: Options,
    },
    Discover(u64),
    AfterLogin(u64),
    Sync(u64),
    SettingReviews(u64),
    ResolveSetting {
        request: u64,
        review: Arc<super::reviews::Review>,
        choice: super::reviews::Choice,
    },
    AutoJoin {
        request: u64,
        review: Arc<Discovery>,
    },
    Create {
        request: u64,
        review: Arc<Discovery>,
        name: String,
        options: Options,
    },
    Resume(u64),
    Stop(u64),
}
impl Request {
    pub fn id(&self) -> u64 {
        match self {
            Self::Status(id)
            | Self::Discover(id)
            | Self::AfterLogin(id)
            | Self::Sync(id)
            | Self::SettingReviews(id)
            | Self::Resume(id)
            | Self::Stop(id) => *id,
            Self::ResolveSetting { request, .. }
            | Self::AutoJoin { request, .. }
            | Self::JoinReview { request, .. }
            | Self::JoinAccept { request, .. }
            | Self::Page { request, .. }
            | Self::Options { request, .. }
            | Self::Create { request, .. }
            | Self::Change { request, .. } => *request,
        }
    }
}
#[derive(Clone, Debug)]
pub enum Update {
    Pending(Arc<Snapshot>),
    Status(Arc<Snapshot>),
    Review(Arc<Discovery>),
    LoginReview(Arc<Discovery>),
    AutoJoined {
        snapshot: Arc<Snapshot>,
        name: String,
        accounts: usize,
        settings: usize,
    },
    Published(Arc<Snapshot>),
    JoinReview(Arc<super::join::Review>),
    Joined(Arc<Snapshot>),
    SettingReviews {
        snapshot: Arc<Snapshot>,
        reviews: Vec<Arc<super::reviews::Review>>,
        saved: bool,
    },
    Synced {
        snapshot: Arc<Snapshot>,
        report: super::continuous::Report,
    },
    Failed(String),
    Stopped,
}
