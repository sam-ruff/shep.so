use super::*;
use crate::store::{RemovalJob, RemovalStage};
#[cfg(test)]
mod tests;

impl Engine {
    pub(in crate::engine) async fn execute_removal_work(
        &self,
        id: String,
        mut output: Output,
    ) -> bool {
        let result = async {
            let job = self.store.removal_job(id.clone()).await?;
            if !matches!(job.stage, RemovalStage::Queued | RemovalStage::Cleanup) {
                return Ok(false);
            }
            if self.bulk_control.stopping.get() {
                return Ok(false);
            }
            let drained = tokio::select! {
                _=self.bulk_control.stopping.requested()=>return Ok(false),
                guard=self.connection_access(&job.target)=>guard,
            };
            drop(drained);
            let _lifecycle = tokio::select! {
                _=self.bulk_control.stopping.requested()=>return Ok(false),
                guard=self.connection_lifecycle.write()=>guard,
            };
            // Admission fences new provider work; the initial drain held no global grant.
            let _owner = tokio::select! {
                _=self.bulk_control.stopping.requested()=>return Ok(false),
                guard=self.connection_access(&job.target)=>guard,
            };
            let current = self.store.removal_job(id.clone()).await?;
            anyhow::ensure!(current == job, "Removal progress changed before cleanup.");
            let job = if job.local_done {
                job
            } else {
                self.store.finish_connection_removal(job).await?
            };
            if !job.local_done {
                let _ = output.send(Event::RemovalChanged(job)).await;
                self.workspace(&mut output).await?;
                return Ok(true);
            }
            let failed = if job.device_credentials {
                self.cleanup_owner(&job.target).await?
            } else {
                0
            };
            let (stage, error) = if failed == 0 {
                (RemovalStage::Succeeded, None)
            } else {
                (
                    RemovalStage::Failed,
                    Some(
                        "Local data is removed. Unlock your credential store, then retry cleanup."
                            .into(),
                    ),
                )
            };
            let saved = self.store.removal_progress(job, stage, error).await?;
            let _ = output.send(Event::RemovalChanged(saved)).await;
            self.workspace(&mut output).await?;
            Ok::<_, anyhow::Error>(true)
        }
        .await;
        match result {
            Ok(progress) => progress,
            Err(_) => {
                if let Ok(job) = self.store.removal_job(id).await
                    && job.stage != RemovalStage::Succeeded
                    && let Ok(saved) = self
                        .store
                        .removal_progress(
                            job,
                            RemovalStage::Failed,
                            Some(
                                "Local cleanup could not finish. Check device storage and retry."
                                    .into(),
                            ),
                        )
                        .await
                {
                    let _ = output.send(Event::RemovalChanged(saved)).await;
                    let _ = self.workspace(&mut output).await;
                }
                false
            }
        }
    }

    pub(in crate::engine) async fn retry_removal(
        &self,
        expected: RemovalJob,
    ) -> anyhow::Result<RemovalJob> {
        anyhow::ensure!(
            expected.stage == RemovalStage::Failed,
            "Refresh removal progress before retrying."
        );
        let stage = if expected.local_done {
            RemovalStage::Cleanup
        } else {
            RemovalStage::Queued
        };
        self.store.removal_progress(expected, stage, None).await
    }
}
