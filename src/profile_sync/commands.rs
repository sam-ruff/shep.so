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
    Options {
        request: u64,
        revision: u64,
        options: Options,
    },
    Discover(u64),
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
            Self::Status(id) | Self::Discover(id) | Self::Resume(id) | Self::Stop(id) => *id,
            Self::Options { request, .. }
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
    Published(Arc<Snapshot>),
    Failed(String),
    Stopped,
}
