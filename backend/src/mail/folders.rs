//! Transient folder observations. Durable intent and recovery stay in the browser.
use super::*;
use serde::Serialize;
use shep_mail_core::folder_actions::creation::CreateOutcome;
use shep_mail_core::{folder_actions::Connection as _, folders::Mailbox};
mod changes;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait FolderProvider: Send + Sync {
    async fn catalog(&self) -> anyhow::Result<Vec<Mailbox>>;
    async fn plan(&self, parent: Option<String>, name: String) -> anyhow::Result<Mailbox>;
    async fn inspect(&self, target: Mailbox) -> anyhow::Result<Option<Mailbox>>;
    async fn create(&self, target: Mailbox) -> CreateOutcome;
    async fn apply(
        &self,
        step: shep_mail_core::folder_actions::Step,
    ) -> shep_mail_core::folder_actions::Outcome;
}

pub(super) struct FolderConnection<
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug,
> {
    pub connection: Mutex<shep_mail_core::providers::mail::folders::ImapFolders<T>>,
}

#[async_trait]
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug>
    FolderProvider for FolderConnection<T>
{
    async fn catalog(&self) -> anyhow::Result<Vec<Mailbox>> {
        self.connection.lock().await.catalog().await
    }
    async fn plan(&self, parent: Option<String>, name: String) -> anyhow::Result<Mailbox> {
        self.connection
            .lock()
            .await
            .plan_folder(parent.as_deref(), &name)
            .await
    }
    async fn inspect(&self, target: Mailbox) -> anyhow::Result<Option<Mailbox>> {
        self.connection
            .lock()
            .await
            .find_planned_folder(&target)
            .await
    }
    async fn create(&self, target: Mailbox) -> CreateOutcome {
        self.connection
            .lock()
            .await
            .create_planned_folder(&target)
            .await
    }
    async fn apply(
        &self,
        step: shep_mail_core::folder_actions::Step,
    ) -> shep_mail_core::folder_actions::Outcome {
        self.connection.lock().await.apply(&step).await
    }
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/mail/folders/catalog", post(catalog))
        .route("/api/mail/folders/plan", post(plan))
        .route("/api/mail/folders/inspect", post(inspect))
        .route("/api/mail/folders/create", post(create))
        .merge(changes::routes())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogRequest {
    connection: Connection,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanRequest {
    connection: Connection,
    parent: Option<String>,
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectRequest {
    connection: Connection,
    target: Mailbox,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Creation {
    Observed { mailbox: Mailbox },
    Acknowledged { target: Mailbox },
    Waiting,
    Rejected,
    Uncertain,
}

async fn create_checked(provider: &dyn FolderProvider, target: Mailbox) -> Creation {
    match provider.inspect(target.clone()).await {
        Ok(Some(mailbox)) => return Creation::Observed { mailbox },
        Ok(None) => {}
        Err(_) => return Creation::Waiting,
    }
    match provider.create(target.clone()).await {
        CreateOutcome::Acknowledged => Creation::Acknowledged { target },
        CreateOutcome::Rejected(_) => Creation::Rejected,
        CreateOutcome::Uncertain(_) => Creation::Uncertain,
    }
}

async fn create(
    State(state): State<AppState>,
    Extension(_permit): Extension<Arc<Admission>>,
    Json(request): Json<InspectRequest>,
) -> Response {
    if let Err((status, message)) = allowed_folder(&state, &request.connection) {
        return fail(status, message);
    }
    if !valid_folder(&request.target.name) {
        return fail(
            StatusCode::BAD_REQUEST,
            "Choose a valid saved folder target.",
        );
    }
    let result = match state.mail.transport.folders(&request.connection).await {
        Ok(provider) => create_checked(provider.as_ref(), request.target).await,
        Err(_) => Creation::Waiting,
    };
    Json(result).into_response()
}

fn allowed_folder(state: &AppState, c: &Connection) -> Result<(), (StatusCode, &'static str)> {
    allowed(state, c, false)?;
    if c.account.protocol != Protocol::Imap {
        return Err((StatusCode::BAD_REQUEST, "POP3 folders stay on this device."));
    }
    Ok(())
}

fn observation<T: Serialize>(result: anyhow::Result<T>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(_) => fail(
            StatusCode::BAD_GATEWAY,
            "Folders could not be checked. Your saved request was kept; reconnect and retry the check.",
        ),
    }
}

fn planned(result: anyhow::Result<Mailbox>) -> Response {
    if result.as_ref().err().is_some_and(|error| {
        error
            .downcast_ref::<shep_mail_core::folder_actions::creation::PlanRejected>()
            .is_some()
    }) {
        return Json(Creation::Rejected).into_response();
    }
    observation(result)
}

async fn read_catalog(provider: &dyn FolderProvider) -> anyhow::Result<Vec<Mailbox>> {
    let catalog = provider.catalog().await?;
    anyhow::ensure!(
        catalog.len() <= 4096,
        "The folder catalog is too large to review."
    );
    Ok(catalog)
}

async fn catalog(
    State(state): State<AppState>,
    Extension(_permit): Extension<Arc<Admission>>,
    Json(request): Json<CatalogRequest>,
) -> Response {
    if let Err((status, message)) = allowed_folder(&state, &request.connection) {
        return fail(status, message);
    }
    observation(
        async {
            let provider = state.mail.transport.folders(&request.connection).await?;
            read_catalog(provider.as_ref()).await
        }
        .await,
    )
}

async fn plan(
    State(state): State<AppState>,
    Extension(_permit): Extension<Arc<Admission>>,
    Json(request): Json<PlanRequest>,
) -> Response {
    if let Err((status, message)) = allowed_folder(&state, &request.connection) {
        return fail(status, message);
    }
    if !valid_folder(&request.name)
        || request
            .parent
            .as_ref()
            .is_some_and(|parent| !valid_folder(parent))
    {
        return fail(
            StatusCode::BAD_REQUEST,
            "Choose a valid folder name and parent.",
        );
    }
    planned(
        async {
            let provider = state.mail.transport.folders(&request.connection).await?;
            provider.plan(request.parent, request.name).await
        }
        .await,
    )
}

async fn inspect(
    State(state): State<AppState>,
    Extension(_permit): Extension<Arc<Admission>>,
    Json(request): Json<InspectRequest>,
) -> Response {
    if let Err((status, message)) = allowed_folder(&state, &request.connection) {
        return fail(status, message);
    }
    if !valid_folder(&request.target.name) {
        return fail(
            StatusCode::BAD_REQUEST,
            "Choose a valid saved folder target.",
        );
    }
    observation(
        async {
            let provider = state.mail.transport.folders(&request.connection).await?;
            provider.inspect(request.target).await
        }
        .await,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn planning_distinguishes_proven_rejection_from_failed_discovery() {
        for rejected in [false, true] {
            let mut provider = MockFolderProvider::new();
            provider.expect_plan().times(1).return_once(move |_, _| {
                if rejected {
                    Err(shep_mail_core::folder_actions::creation::PlanRejected(
                        "private provider detail".into(),
                    )
                    .into())
                } else {
                    Err(anyhow::anyhow!("private discovery failure"))
                }
            });
            provider.expect_create().never();
            let response = planned(provider.plan(None, "Projects".into()).await);
            assert_eq!(
                response.status(),
                if rejected {
                    StatusCode::OK
                } else {
                    StatusCode::BAD_GATEWAY
                }
            );
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .expect("bounded body");
            let body = String::from_utf8(body.to_vec()).expect("JSON");
            assert!(!body.contains("private"));
            if rejected {
                assert_eq!(body, r#"{"state":"rejected"}"#);
            }
        }
    }

    #[tokio::test]
    async fn catalog_keeps_exact_names_and_metadata() {
        let mut provider = MockFolderProvider::new();
        let folder = Mailbox::flat("Projects.with.dot".into());
        let expected = folder.clone();
        provider
            .expect_catalog()
            .times(1)
            .return_once(move || Ok(vec![folder]));
        assert_eq!(read_catalog(&provider).await.expect("catalog"), [expected]);
    }

    #[tokio::test]
    async fn catalog_failure_and_oversized_review_are_not_partial_success() {
        let mut provider = MockFolderProvider::new();
        provider
            .expect_catalog()
            .times(1)
            .return_once(|| Err(anyhow::anyhow!("synthetic provider secret")));
        assert!(read_catalog(&provider).await.is_err());
        let mut provider = MockFolderProvider::new();
        provider
            .expect_catalog()
            .times(1)
            .return_once(|| Ok(vec![Mailbox::flat("Fixture".into()); 4097]));
        assert!(read_catalog(&provider).await.is_err());
    }

    #[tokio::test]
    async fn existing_target_and_failed_inspection_never_dispatch_create() {
        let target = Mailbox::flat("Fixture".into());
        let mut provider = MockFolderProvider::new();
        let observed = target.clone();
        provider
            .expect_inspect()
            .times(1)
            .return_once(move |_| Ok(Some(observed)));
        assert_eq!(
            create_checked(&provider, target.clone()).await,
            Creation::Observed {
                mailbox: target.clone()
            }
        );
        let mut provider = MockFolderProvider::new();
        provider
            .expect_inspect()
            .times(1)
            .return_once(|_| Err(anyhow::anyhow!("private preflight failure")));
        assert_eq!(create_checked(&provider, target).await, Creation::Waiting);
    }

    #[tokio::test]
    async fn creation_acknowledgement_is_returned_without_a_later_catalog_read() {
        let target = Mailbox::flat("Fixture".into());
        let mut provider = MockFolderProvider::new();
        provider.expect_inspect().times(1).return_once(|_| Ok(None));
        provider
            .expect_create()
            .with(mockall::predicate::eq(target.clone()))
            .times(1)
            .return_once(|_| CreateOutcome::Acknowledged);
        assert_eq!(
            create_checked(&provider, target.clone()).await,
            Creation::Acknowledged { target }
        );
    }

    #[tokio::test]
    async fn definite_refusal_and_unknown_outcome_remain_distinct() {
        for (outcome, expected) in [
            (
                CreateOutcome::Rejected("private refusal".into()),
                Creation::Rejected,
            ),
            (
                CreateOutcome::Uncertain("private response lost".into()),
                Creation::Uncertain,
            ),
        ] {
            let mut provider = MockFolderProvider::new();
            provider.expect_inspect().times(1).return_once(|_| Ok(None));
            provider
                .expect_create()
                .times(1)
                .return_once(move |_| outcome);
            assert_eq!(
                create_checked(&provider, Mailbox::flat("Fixture".into())).await,
                expected
            );
        }
    }
}
