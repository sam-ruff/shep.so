//! Background bridge between verified Drive records and the shared causal worker.
//! Enrollment owns consent/lifecycle and account application. This bridge never
//! reads credentials, mutates mail accounts or chooses a conflict winner.
use super::*;
use shep_profile_core::history::{self, Command, Reply, State, Worker};
use std::path::PathBuf;

/// The enrollment review must persist this choice before a publishing pass.
/// An empty Drive list never implicitly authorizes creating a new profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishIntent {
    ExistingProfile,
    ReviewedNewProfile,
}

/// One history owner; async mutable methods serialize passes without a shared
/// state mutex. Each history/transport journal has its own bounded FIFO worker.
pub struct Replica {
    binding: history::Binding,
    history: Worker,
    journal: journal::Journal,
}

/// Minted after a complete verified pull and causal drain. A different device,
/// newer local edit or replacement scan must obtain a fresh proof before push.
#[derive(Debug)]
pub struct Pulled {
    scan: journal::Scan,
    state: State,
    published: bool,
}
impl Pulled {
    pub fn state(&self) -> &State {
        &self.state
    }
    pub fn remote_records(&self) -> u64 {
        self.scan.files()
    }
}

impl Replica {
    pub async fn open(
        path: PathBuf,
        binding: history::Binding,
        journal: journal::Journal,
    ) -> anyhow::Result<Self> {
        Binding::new(binding.principal.clone(), binding.namespace.clone())?;
        let history = Worker::open(path, binding.clone()).await?;
        Ok(Self {
            binding,
            history,
            journal,
        })
    }

    pub fn binding(&self) -> &history::Binding {
        &self.binding
    }

    pub async fn state(&self) -> anyhow::Result<State> {
        state(self.history.request(Command::State).await?)
    }

    /// Generate the edit UUID once and reuse the exact request after a lost
    /// acknowledgment. The shared worker checks revisions/conflicts/extensions.
    pub async fn edit(&mut self, edit: history::LocalEdit) -> anyhow::Result<State> {
        state(self.history.request(Command::Edit { edit }).await?)
    }

    /// Own local admission through its field check. A replay cannot upgrade the
    /// native baseline past an unseen remote edit merely because Edit succeeded.
    pub async fn admit_local(
        &mut self,
        pending: super::state::Pending,
    ) -> anyhow::Result<super::state::Admitted> {
        anyhow::ensure!(
            self.state().await?.initialized,
            "Finish profile setup on its original device before sharing local changes."
        );
        let current = self.edit(pending.edit()).await?;
        let versions = self.versions(pending.target(), None).await?;
        anyhow::ensure!(
            current.initialized
                && !current.removed
                && current.waiting == 0
                && current.ready == 0
                && versions.len() == 1
                && versions[0].operation == pending.operation,
            history::Error::Changed
        );
        anyhow::ensure!(
            self.value(pending.target(), pending.operation).await? == pending.change,
            "The admitted profile edit differs from its saved local request."
        );
        Ok(super::state::Admitted {
            binding: self.binding.clone(),
            pending,
            revision: current.revision,
        })
    }

    /// Review pages stay bounded. Callers must not apply a partial/conflicted
    /// history to accounts; retain State/revision through the application review.
    pub async fn fields(&self, after: Option<String>) -> anyhow::Result<Vec<history::Field>> {
        match self.history.request(Command::Fields { after }).await? {
            Reply::Fields(fields) => Ok(fields),
            _ => anyhow::bail!("The profile worker returned an unexpected field page."),
        }
    }
    pub(crate) async fn observe(
        &self,
        after: Option<String>,
    ) -> anyhow::Result<super::continuous::Observed> {
        let fields = self.fields(after).await?;
        let state = self.state().await?;
        anyhow::ensure!(
            state.initialized && !state.removed && state.ready == 0 && state.waiting == 0,
            history::Error::Incomplete
        );
        let mut values = Vec::with_capacity(fields.len());
        for field in fields {
            let change = if field.conflict {
                None
            } else {
                let versions = self.versions(field.target.clone(), None).await?;
                anyhow::ensure!(!versions.is_empty(), "A shared field has no current value.");
                Some(
                    self.value(field.target.clone(), versions[0].operation)
                        .await?,
                )
            };
            values.push((field, change));
        }
        Ok(super::continuous::Observed {
            binding: self.binding.clone(),
            revision: state.revision,
            fields: values,
        })
    }
    pub(crate) async fn next_upload_changes(
        &self,
    ) -> anyhow::Result<Option<Vec<shep_profile_core::Change>>> {
        match self.history.request(Command::NextUpload).await? {
            Reply::Upload(Some(upload)) => Ok(Some(
                shep_profile_core::Operation::decode(upload.record.as_bytes())?.changes,
            )),
            Reply::Upload(None) => Ok(None),
            _ => anyhow::bail!("The profile worker returned an unexpected queued edit."),
        }
    }
    pub async fn versions(
        &self,
        target: String,
        after: Option<Uuid>,
    ) -> anyhow::Result<Vec<history::Version>> {
        match self
            .history
            .request(Command::Versions { target, after })
            .await?
        {
            Reply::Versions(versions) => Ok(versions),
            _ => anyhow::bail!("The profile worker returned an unexpected version page."),
        }
    }
    pub async fn value(
        &self,
        target: String,
        operation: Uuid,
    ) -> anyhow::Result<shep_profile_core::Change> {
        match self
            .history
            .request(Command::Value { target, operation })
            .await?
        {
            Reply::Value(value) => Ok(value),
            _ => anyhow::bail!("The profile worker returned an unexpected field value."),
        }
    }

    fn check_session(&self, session: &drive::Session) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.binding.namespace == session.binding().namespace()
                && self.binding.principal == session.binding().identity(),
            "The profile belongs to another Google account or application. Reconnect its original account."
        );
        Ok(())
    }

    /// Resume interrupted listing; refresh a completed listing. Download only
    /// after listing completes. Import one bounded record at a time; retrying
    /// after cancellation/restart replays exact bytes idempotently.
    pub async fn pull(&mut self, session: &drive::Session) -> anyhow::Result<Pulled> {
        self.pull_controlled(session, &control::Control::default())
            .await
    }
    pub async fn pull_controlled(
        &mut self,
        session: &drive::Session,
        control: &control::Control,
    ) -> anyhow::Result<Pulled> {
        control.check()?;
        self.check_session(session)?;
        let scope = Some((self.binding.profile, self.binding.generation));
        let mut scan = match self.journal.resume_scan(session.binding(), scope).await? {
            Some(scan) if !scan.complete() => scan,
            _ => {
                self.journal
                    .begin_scan(session.binding().clone(), scope)
                    .await?
            }
        };
        while !scan.complete() {
            let page = control.read(session.page_for(&scan)).await?;
            scan = self.journal.append_page(&scan, page).await?;
        }
        let mut after = None;
        loop {
            control.check()?;
            let page = self.journal.scan_entries(&scan, after).await?;
            if page.is_empty() {
                break;
            }
            for entry in page {
                let record = match self.journal.cached_download(&scan, &entry.record).await? {
                    Some(record) => record,
                    None => {
                        let record = control.read(session.download(&entry.record)).await?;
                        self.journal
                            .cache_download(&scan, &entry.record, record)
                            .await?
                    }
                };
                self.history
                    .request(Command::Import {
                        record: String::from_utf8(record.bytes().to_vec())?,
                    })
                    .await?;
                after = Some(entry.position);
            }
        }
        let mut current = self.state().await?;
        while current.ready != 0 {
            control.check()?;
            current = state(self.history.request(Command::Drain).await?)?;
        }
        anyhow::ensure!(current.waiting == 0, history::Error::Incomplete);
        Ok(Pulled {
            scan,
            state: current,
            published: false,
        })
    }

    /// Publish one durable edit per call, allowing an owning coordinator to
    /// observe stop/category changes between requests. Once upload starts, keep
    /// this future owned through its durable acknowledgment, even on UI close.
    pub async fn publish_next(
        &mut self,
        session: &drive::Session,
        pulled: &mut Pulled,
        intent: PublishIntent,
    ) -> anyhow::Result<bool> {
        self.check_session(session)?;
        let current = self.state().await?;
        anyhow::ensure!(
            current.device == pulled.state.device
                && current.revision == pulled.state.revision
                && pulled.scan.binding() == session.binding()
                && pulled.scan.profile() == Some((self.binding.profile, self.binding.generation)),
            "The profile changed after discovery. Pull again before publishing its saved edits."
        );
        anyhow::ensure!(
            current.waiting == 0 && current.ready == 0,
            history::Error::Incomplete
        );
        if pulled.scan.files() == 0 && !pulled.published {
            anyhow::ensure!(
                intent == PublishIntent::ReviewedNewProfile && current.operations == current.queued,
                "The existing cloud profile is missing. Check the Google application or restore it; the local profile was not uploaded as a new one."
            );
        }
        let pending = match self.history.request(Command::NextUpload).await? {
            Reply::Upload(Some(upload)) => upload,
            Reply::Upload(None) => return Ok(false),
            _ => anyhow::bail!("The profile worker returned an unexpected queued edit."),
        };
        let record = Record::decode(&self.binding.namespace, pending.record.into_bytes())?;
        anyhow::ensure!(
            record.sha256 == pending.sha256
                && record.key()
                    == Key {
                        profile: self.binding.profile,
                        generation: self.binding.generation,
                        operation: pending.operation,
                    },
            "The saved profile edit has inconsistent identity or bytes. Keep it for review."
        );
        // This query also verifies that discovery is still the current complete
        // scan, including when no matching operation exists remotely.
        let observed = self.journal.scan_record(&pulled.scan, record.key()).await?;
        let saved = self.journal.load(session.binding(), record.key()).await?;
        let mut id = pending.file_id;
        for candidate in observed
            .as_ref()
            .map(|r| r.id())
            .into_iter()
            .chain(saved.as_ref().map(|s| s.upload().remote().id()))
        {
            anyhow::ensure!(
                id.as_deref().is_none_or(|id| id == candidate),
                "The same profile operation has different Drive identities. Keep the saved reservation and review discovery; no duplicate was uploaded."
            );
            id = Some(candidate.to_owned());
        }
        if let Some(observed) = &observed {
            observed.verify(&record)?;
        }
        let upload = match id {
            Some(id) => ReservedUpload {
                binding: session.binding().clone(),
                remote: RemoteRecord {
                    id,
                    key: record.key(),
                    size: record.bytes().len() as u64,
                    sha256: record.sha256.clone(),
                },
                record,
            },
            None => session.reserve(record).await?,
        };
        upload.validate()?;
        // Commit the core's original ID before obtaining the transport proof.
        // Either acknowledgment can be lost; retry always converges on this ID.
        self.history
            .request(Command::Reserve {
                operation: pending.operation,
                file_id: upload.remote.id.clone(),
            })
            .await?;
        let durable = self.journal.prepare(upload).await?;
        let receipt = session.upload(&durable).await?;
        self.journal.acknowledge(&durable, &receipt).await?;
        pulled.state = state(
            self.history
                .request(Command::Confirm {
                    operation: pending.operation,
                    file_id: receipt.id,
                    sha256: receipt.sha256,
                })
                .await?,
        )?;
        pulled.published = true;
        Ok(true)
    }

    pub async fn close(self) -> anyhow::Result<()> {
        self.history.close().await?;
        Ok(())
    }
}

fn state(reply: Reply) -> anyhow::Result<State> {
    match reply {
        Reply::State(state) => Ok(state),
        _ => anyhow::bail!("The profile worker returned an unexpected state."),
    }
}

#[cfg(test)]
mod tests;
