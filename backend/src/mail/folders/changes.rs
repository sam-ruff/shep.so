use super::*;
use shep_mail_core::folder_actions::{Action, Job, Outcome, Plan, Progress, Review, Status, Step};
use shep_mail_core::folders::Tree;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/mail/folders/review", post(review))
        .route("/api/mail/folders/step", post(apply))
        .route("/api/mail/folders/check-step", post(check))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewRequest {
    connection: Connection,
    source: String,
    action: Action,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepRequest {
    connection: Connection,
    plan: Plan,
    completed: usize,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Applied {
    Acknowledged { step: Step },
    Observed { step: Step },
    Waiting,
    Rejected,
    Uncertain,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Checked {
    Applied { catalog: Vec<Mailbox> },
    Original { catalog: Vec<Mailbox> },
    Changed { catalog: Vec<Mailbox> },
}

fn checked_plan(plan: &Plan) -> anyhow::Result<()> {
    anyhow::ensure!(
        !plan.members.is_empty() && plan.members.len() <= 128,
        "Review at most 128 folders at once."
    );
    anyhow::ensure!(valid_folder(&plan.source), "Invalid source folder.");
    let mut original = Vec::new();
    for member in &plan.members {
        anyhow::ensure!(
            valid_folder(&member.mailbox.name)
                && valid_folder(&member.path)
                && member
                    .destination
                    .as_ref()
                    .is_none_or(|name| valid_folder(name)),
            "Invalid folder path."
        );
        if member.listed {
            original.push(member.mailbox.clone());
        }
    }
    if let Some(parent) = &plan.parent {
        original.push(parent.clone());
    }
    let canonical = Plan::new(&Tree::new(&original), &plan.source, plan.action.clone())?;
    anyhow::ensure!(&canonical == plan, "The saved subtree review is invalid.");
    Ok(())
}

fn job(plan: Plan, completed: usize) -> anyhow::Result<Job> {
    checked_plan(&plan)?;
    let steps = plan.steps();
    anyhow::ensure!(
        completed < steps.len(),
        "This subtree request has no remaining step."
    );
    Ok(Job {
        id: String::new(),
        query_counts: None,
        revision: 0,
        label: plan.source.clone(),
        destination_label: None,
        review: Review {
            account: String::new(),
            connection: String::new(),
            imap: true,
            plan,
            cached_messages: 0,
            affected_history: 0,
        },
        steps: steps
            .into_iter()
            .enumerate()
            .map(|(position, step)| Progress {
                position,
                step,
                status: if position < completed {
                    Status::Done
                } else {
                    Status::Queued
                },
                error: None,
            })
            .collect(),
        closed: false,
    })
}

#[derive(Serialize)]
struct Reviewed {
    plan: Plan,
    catalog: Vec<Mailbox>,
}
async fn reviewed(
    provider: &dyn FolderProvider,
    source: String,
    action: Action,
) -> anyhow::Result<Reviewed> {
    anyhow::ensure!(valid_folder(&source), "Invalid source folder.");
    let catalog = read_catalog(provider).await?;
    let plan = Plan::new(&Tree::new(&catalog), &source, action)?;
    checked_plan(&plan)?;
    Ok(Reviewed { plan, catalog })
}

async fn apply_checked(provider: &dyn FolderProvider, plan: Plan, completed: usize) -> Applied {
    let Ok(job) = job(plan, completed) else {
        return Applied::Rejected;
    };
    let Ok(catalog) = read_catalog(provider).await else {
        return Applied::Waiting;
    };
    let Ok(absent) = job.preflight(&catalog) else {
        return Applied::Rejected;
    };
    let step = job.steps[completed].step.clone();
    if matches!(&step, Step::Forget { .. })
        || matches!(&step, Step::Delete { source } if absent.contains(source))
    {
        return Applied::Observed { step };
    }
    match provider.apply(step.clone()).await {
        Outcome::Applied => Applied::Acknowledged { step },
        Outcome::Rejected(_) => Applied::Rejected,
        Outcome::Uncertain(_) => Applied::Uncertain,
    }
}

async fn check_step(
    provider: &dyn FolderProvider,
    plan: Plan,
    completed: usize,
) -> anyhow::Result<Checked> {
    let job = job(plan, completed)?;
    let catalog = read_catalog(provider).await?;
    let applied = match &job.steps[completed].step {
        Step::Rename { .. } => {
            job.review.plan.members.iter().all(|member| {
                !catalog
                    .iter()
                    .any(|mailbox| mailbox.name == member.mailbox.name)
            }) && job.confirm_renamed_catalog(&catalog).is_ok()
        }
        Step::Delete { source } | Step::Forget { source } => {
            !catalog.iter().any(|mailbox| &mailbox.name == source)
        }
    };
    if applied {
        return Ok(Checked::Applied { catalog });
    }
    if job.preflight(&catalog).is_ok() {
        return Ok(Checked::Original { catalog });
    }
    Ok(Checked::Changed { catalog })
}

async fn review(
    State(state): State<AppState>,
    Extension(_permit): Extension<Arc<Admission>>,
    Json(request): Json<ReviewRequest>,
) -> Response {
    if let Err((status, message)) = allowed_folder(&state, &request.connection) {
        return fail(status, message);
    }
    if !valid_folder(&request.source) {
        return fail(StatusCode::BAD_REQUEST, "Choose a valid source folder.");
    }
    observation(
        async {
            let provider = state.mail.transport.folders(&request.connection).await?;
            reviewed(provider.as_ref(), request.source, request.action).await
        }
        .await,
    )
}
async fn apply(
    State(state): State<AppState>,
    Extension(_permit): Extension<Arc<Admission>>,
    Json(request): Json<StepRequest>,
) -> Response {
    if let Err((status, message)) = allowed_folder(&state, &request.connection) {
        return fail(status, message);
    }
    if checked_plan(&request.plan).is_err() || request.completed >= request.plan.steps().len() {
        return fail(
            StatusCode::BAD_REQUEST,
            "Review a valid remaining folder step.",
        );
    }
    let Ok(provider) = state.mail.transport.folders(&request.connection).await else {
        return Json(Applied::Waiting).into_response();
    };
    Json(apply_checked(provider.as_ref(), request.plan, request.completed).await).into_response()
}
async fn check(
    State(state): State<AppState>,
    Extension(_permit): Extension<Arc<Admission>>,
    Json(request): Json<StepRequest>,
) -> Response {
    if let Err((status, message)) = allowed_folder(&state, &request.connection) {
        return fail(status, message);
    }
    if checked_plan(&request.plan).is_err() || request.completed >= request.plan.steps().len() {
        return fail(
            StatusCode::BAD_REQUEST,
            "Review a valid remaining folder step.",
        );
    }
    observation(
        async {
            let provider = state.mail.transport.folders(&request.connection).await?;
            check_step(provider.as_ref(), request.plan, request.completed).await
        }
        .await,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn catalog() -> Vec<Mailbox> {
        ["Projects", "Projects/Child", "Archive"]
            .into_iter()
            .map(|name| Mailbox {
                name: name.into(),
                delimiter: Some('/'),
                encoding: shep_mail_core::folders::NameEncoding::Utf8,
                selectable: true,
                no_inferiors: false,
                non_existent: false,
                role: None,
            })
            .collect()
    }
    fn deletion() -> Plan {
        Plan::new(&Tree::new(&catalog()), "Projects", Action::Delete).expect("fixture plan")
    }
    #[tokio::test]
    async fn changed_subtree_or_failed_catalog_cannot_dispatch() {
        for offline in [true, false] {
            let mut provider = MockFolderProvider::new();
            provider.expect_catalog().times(1).returning(move || {
                if offline {
                    Err(anyhow::anyhow!("offline"))
                } else {
                    let mut values = catalog();
                    values.push(Mailbox {
                        name: "Projects/New".into(),
                        ..values[0].clone()
                    });
                    Ok(values)
                }
            });
            provider.expect_apply().never();
            assert_eq!(
                apply_checked(&provider, deletion(), 0).await,
                if offline {
                    Applied::Waiting
                } else {
                    Applied::Rejected
                }
            );
        }
    }
    #[tokio::test]
    async fn acknowledged_step_does_not_wait_for_another_catalog() {
        let mut provider = MockFolderProvider::new();
        provider
            .expect_catalog()
            .times(1)
            .returning(|| Ok(catalog()));
        let step = deletion().steps()[0].clone();
        provider
            .expect_apply()
            .with(mockall::predicate::eq(step.clone()))
            .times(1)
            .returning(|_| Outcome::Applied);
        assert_eq!(
            apply_checked(&provider, deletion(), 0).await,
            Applied::Acknowledged { step }
        );
    }
    #[tokio::test]
    async fn read_only_unknown_check_never_deletes_and_requires_complete_catalog() {
        let mut provider = MockFolderProvider::new();
        provider.expect_catalog().times(1).returning(|| {
            Ok(catalog()
                .into_iter()
                .filter(|mailbox| mailbox.name != "Projects/Child")
                .collect())
        });
        provider.expect_apply().never();
        assert!(matches!(
            check_step(&provider, deletion(), 0).await.expect("check"),
            Checked::Applied { .. }
        ));
        let mut failed = MockFolderProvider::new();
        failed
            .expect_catalog()
            .times(1)
            .returning(|| Err(anyhow::anyhow!("incomplete list")));
        failed.expect_apply().never();
        assert!(check_step(&failed, deletion(), 0).await.is_err());
    }
    #[test]
    fn malformed_client_plan_is_rejected_without_panicking() {
        let mut plan = deletion();
        plan.members[0].path = "different".into();
        assert!(checked_plan(&plan).is_err());
        plan.members.clear();
        assert!(checked_plan(&plan).is_err());
    }
    #[tokio::test]
    async fn definite_refusal_and_lost_wire_reply_do_not_become_acknowledgements() {
        for uncertain in [true, false] {
            let mut provider = MockFolderProvider::new();
            provider
                .expect_catalog()
                .times(1)
                .returning(|| Ok(catalog()));
            provider.expect_apply().times(1).returning(move |_| {
                if uncertain {
                    Outcome::Uncertain("lost reply".into())
                } else {
                    Outcome::Rejected("refused".into())
                }
            });
            assert_eq!(
                apply_checked(&provider, deletion(), 0).await,
                if uncertain {
                    Applied::Uncertain
                } else {
                    Applied::Rejected
                }
            );
        }
    }
    #[tokio::test]
    async fn deletion_rechecks_the_remaining_subtree_after_each_receipt() {
        let mut provider = MockFolderProvider::new();
        provider.expect_catalog().times(1).returning(|| {
            Ok(catalog()
                .into_iter()
                .filter(|mailbox| mailbox.name != "Projects/Child")
                .collect())
        });
        provider
            .expect_apply()
            .with(mockall::predicate::eq(Step::Delete {
                source: "Projects".into(),
            }))
            .times(1)
            .returning(|_| Outcome::Applied);
        assert_eq!(
            apply_checked(&provider, deletion(), 1).await,
            Applied::Acknowledged {
                step: Step::Delete {
                    source: "Projects".into()
                }
            }
        );
    }
}
