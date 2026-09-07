//! FIFO, provider-independent selection work. Only bounded observations cross to iced.
use super::*;
use crate::store::{MailSelectionId, SelectionChange, SelectionSnapshot};

#[derive(Debug, Clone)]
pub enum Request {
    Capture(MailSelectionId, MailQuery),
    Change(MailSelectionId, u64, SelectionChange),
    Observe(MailSelectionId),
    Release(MailSelectionId),
}
impl Request {
    pub fn id(&self) -> MailSelectionId {
        match self {
            Self::Capture(id, _) | Self::Change(id, ..) | Self::Observe(id) | Self::Release(id) => {
                *id
            }
        }
    }
}

impl Engine {
    pub(super) async fn selection(
        &self,
        serial: u64,
        request: Request,
        visible: Vec<String>,
        mut output: Output,
    ) -> anyhow::Result<()> {
        let result: anyhow::Result<Option<SelectionSnapshot>> = match request {
            Request::Capture(id, query) => self
                .store
                .capture_selection(id, 0, query, false, visible)
                .await
                .map(Some),
            Request::Change(id, revision, change) => self
                .store
                .change_selection(id, revision, change, visible)
                .await
                .map(Some),
            Request::Observe(id) => self.store.selection_snapshot(id, visible).await.map(Some),
            Request::Release(id) => self.store.release_selection(id).await.map(|()| None),
        };
        output
            .send(Event::Selection(
                serial,
                result
                    .map(|s| s.map(Arc::new))
                    .map_err(|e| format!("{e:#}")),
            ))
            .await?;
        Ok(())
    }
}
