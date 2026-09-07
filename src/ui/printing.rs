use super::*;

#[derive(Debug, Clone)]
pub enum Message {
    Open,
    Launched(u64, Result<(), String>),
}
#[derive(Default)]
pub(super) struct State {
    pub pending: bool,
    pub revision: u64,
    pub source: Option<String>,
    preview: Option<Arc<crate::printing::Preview>>,
    error: Option<String>,
}

impl App {
    pub(super) fn handle_print(&mut self, message: Message) -> Task<super::Message> {
        match message {
            Message::Open => {
                if self.tab != Tab::Mail || self.dialog.is_some() || self.printing.pending {
                    return Task::none();
                }
                let Some(detail) = self
                    .detail
                    .as_ref()
                    .filter(|detail| self.reader_id() == Some(detail.summary.id.as_str()))
                else {
                    return Task::none();
                };
                let source = detail.summary.id.clone();
                let options = crate::printing::Options {
                    plain: !self.formatted(detail),
                    images: if crate::remote_images::allowed(&self.preferences, &detail.summary) {
                        self.remote_bytes.iter().cloned().collect()
                    } else {
                        Default::default()
                    },
                };
                self.printing.revision += 1;
                self.printing.preview = None;
                if self.try_command(Command::Print(
                    self.printing.revision,
                    source.clone(),
                    options,
                )) {
                    self.printing.pending = true;
                    self.printing.source = Some(source);
                    if let Some(error) = self.printing.error.take()
                        && self.notice.as_ref().is_some_and(|n| n.1 && n.0 == error)
                    {
                        self.notice = None;
                    }
                }
            }
            Message::Launched(revision, result) if revision == self.printing.revision => {
                self.printing.pending = false;
                if let Err(error) = result {
                    self.printing.preview = None;
                    self.printing.error = Some(error.clone());
                    self.notice(error, true);
                }
            }
            Message::Launched(..) => {}
        }
        Task::none()
    }
    pub(super) fn print_ready(
        &mut self,
        revision: u64,
        result: Result<Arc<crate::printing::Preview>, String>,
    ) -> Task<super::Message> {
        if revision != self.printing.revision || !self.printing.pending {
            return Task::none();
        }
        match result {
            Ok(preview) => {
                self.printing.preview = Some(preview.clone());
                let demo = self.demo;
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || crate::printing::open(&preview, demo))
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|result| result.map_err(|e| e.to_string()))
                    },
                    move |result| super::Message::Print(Message::Launched(revision, result)),
                )
            }
            Err(error) => self.handle_print(Message::Launched(revision, Err(error))),
        }
    }
}
