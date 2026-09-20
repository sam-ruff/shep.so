use super::*;
use chrono::{DateTime, Utc};
use shep_calendar_core::http::CalendarProvider;
use shep_calendar_core::{Event, FailureKind, Mutation, ProviderFailure, Receipt};

pub(super) fn routes() -> Router<AppState> {
    Router::new().route(
        "/api/calendar",
        post(handle).layer(axum::extract::DefaultBodyLimit::max(192 * 1024)),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    binding: String,
    operation: Operation,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Sources,
    Events {
        source_id: String,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    },
    Mutate {
        request_id: String,
        mutation: Mutation,
    },
    Inspect {
        request_id: String,
        mutation: Mutation,
    },
}

fn reject(message: &str) -> ProviderFailure {
    ProviderFailure::rejected(message)
}

fn validate_mutation(request_id: &str, mutation: &Mutation) -> Result<(), ProviderFailure> {
    if request_id.len() != 36
        || !request_id.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()
            }
        })
    {
        return Err(reject("The saved calendar request identity is invalid."));
    }
    let valid_event = |event: &Event| event.is_bounded() && event.end > event.start;
    match mutation {
        Mutation::Save {
            before: None,
            after,
        } if valid_event(after) && after.is_create() => Ok(()),
        Mutation::Save {
            before: Some(before),
            after,
        } if valid_event(before)
            && valid_event(after)
            && before.id == after.id
            && before.source_id == after.source_id
            && before.etag == after.etag
            && before.remote_url == after.remote_url
            && before.etag.as_ref().is_some_and(|value| !value.is_empty()) =>
        {
            Ok(())
        }
        Mutation::Delete { before }
            if valid_event(before)
                && before.etag.as_ref().is_some_and(|value| !value.is_empty()) =>
        {
            Ok(())
        }
        _ => Err(reject(
            "The event identity changed. Refresh it before making another change.",
        )),
    }
}

async fn access_token(
    state: &AppState,
    session: &Session,
    headers: &HeaderMap,
    write: bool,
) -> Result<Zeroizing<String>, ProviderFailure> {
    let permission = if write {
        grants::Permission::CalendarWrite
    } else {
        grants::Permission::CalendarRead
    };
    grants::access_token(state, session, headers, permission)
        .await
        .map_err(|(_, message)| ProviderFailure::waiting(message))
}

async fn perform(
    provider: &dyn CalendarProvider,
    token: &str,
    operation: Operation,
) -> Result<Value, ProviderFailure> {
    match operation {
        Operation::Sources => Ok(serde_json::json!({"sources":provider.sources(token).await?})),
        Operation::Events {
            source_id,
            start,
            end,
        } => {
            if source_id.is_empty()
                || source_id.len() > 1024
                || end <= start
                || end - start > chrono::Duration::days(366)
            {
                return Err(reject("Choose a calendar window of at most one year."));
            }
            let source = provider
                .sources(token)
                .await?
                .into_iter()
                .find(|source| source.id == source_id)
                .ok_or_else(|| reject("This calendar is no longer available."))?;
            Ok(
                serde_json::json!({"source":source,"events":provider.events(token, &source, start, end).await?}),
            )
        }
        Operation::Mutate {
            request_id,
            mutation,
        } => {
            validate_mutation(&request_id, &mutation)?;
            let source = provider
                .sources(token)
                .await
                .map_err(|error| {
                    ProviderFailure::waiting(format!(
                        "Could not confirm the calendar before dispatch: {}",
                        error.message
                    ))
                })?
                .into_iter()
                .find(|source| source.id == mutation.source_id())
                .ok_or_else(|| reject("This calendar is no longer available."))?;
            if source.read_only {
                return Err(reject("This calendar is read-only."));
            }
            let receipt = match mutation {
                Mutation::Save { before, after } => Receipt {
                    request_id: request_id.clone(),
                    before,
                    after: Some(provider.save(token, &request_id, &after).await?),
                },
                Mutation::Delete { before } => {
                    provider.delete(token, &before).await?;
                    Receipt {
                        request_id,
                        before: Some(before),
                        after: None,
                    }
                }
            };
            Ok(serde_json::json!({"receipt":receipt}))
        }
        Operation::Inspect {
            request_id,
            mutation,
        } => {
            validate_mutation(&request_id, &mutation)?;
            let mut expected = match mutation {
                Mutation::Save {
                    before: None,
                    mut after,
                } => {
                    after.id = format!("shep{}", request_id.replace('-', ""));
                    after
                }
                Mutation::Save {
                    before: Some(before),
                    ..
                }
                | Mutation::Delete { before } => before,
            };
            expected.remote_url = Some(expected.id.clone());
            let current = provider.read(token, &expected).await?;
            if current.as_ref().is_some_and(|event| {
                event.id != expected.id || event.source_id != expected.source_id
            }) {
                return Err(reject(
                    "Google returned a different event identity. Keep the saved change for review.",
                ));
            }
            Ok(serde_json::json!({"current":current}))
        }
    }
}

async fn handle(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Response {
    let mutating = matches!(&request.operation, Operation::Mutate { .. });
    let result = async {
        let binding = hash(&format!(
            "{}\0{}",
            state.config.google_client_id, session.identity.subject
        ));
        if request.binding != binding {
            return Err(reject(
                "This Calendar request belongs to a different signed-in identity.",
            ));
        }
        if let Operation::Mutate {
            request_id,
            mutation,
        }
        | Operation::Inspect {
            request_id,
            mutation,
        } = &request.operation
        {
            validate_mutation(request_id, mutation)?;
        }
        let provider = state
            .calendar
            .as_ref()
            .ok_or_else(|| reject("Calendar is unavailable on this gateway."))?;
        let _slot = state.calendar_slots.try_acquire().map_err(|_| {
            ProviderFailure::waiting("Calendar is busy. The saved change can wait.")
        })?;
        let write = matches!(&request.operation, Operation::Mutate { .. });
        let token = access_token(&state, &session, &headers, write).await?;
        perform(provider.as_ref(), &token, request.operation).await
    }
    .await;
    match result {
        Ok(value) => Json(serde_json::json!({"state":if mutating {"acknowledged"} else {"observed"},"value":value})).into_response(),
        Err(error) => {
            let kind = match error.kind { FailureKind::Waiting => "waiting", FailureKind::Rejected => "rejected", FailureKind::Uncertain => "uncertain" };
            Json(serde_json::json!({"state":kind,"error":error.message})).into_response()
        }
    }
}

#[cfg(test)]
mod tests;
