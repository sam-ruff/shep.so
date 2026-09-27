use super::{GoogleCalendarProvider, bounded_text, google_event, response_failure};
use crate::restoration::{
    DeletePlan, DeleteReceipt, Inspection, LiveStatus, RestoreRequest, strong_etag,
};
use crate::{Event, ProviderFailure};
use serde_json::{Value, json};

fn rejected() -> ProviderFailure {
    ProviderFailure::rejected(
        "This event cannot be restored safely. Refresh and review it in Google Calendar.",
    )
}

fn identity(value: &Value, expected: &Event) -> Result<(Event, String, String), ProviderFailure> {
    if value["id"].as_str() != Some(expected.id.as_str())
        || value["organizer"]["self"].as_bool() != Some(true)
        || value.get("recurrence").is_some()
        || value.get("recurringEventId").is_some()
        || value.get("originalStartTime").is_some()
        || !value["etag"].as_str().is_some_and(strong_etag)
    {
        return Err(rejected());
    }
    let uid = value["iCalUID"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 1024)
        .ok_or_else(rejected)?;
    let organiser = value["organizer"]["email"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 1024)
        .ok_or_else(rejected)?;
    let event = google_event(value, &expected.source_id).map_err(|_| rejected())?;
    if event.end <= event.start {
        return Err(rejected());
    }
    Ok((event, uid.into(), organiser.into()))
}

fn live_status(value: &Value) -> Result<LiveStatus, ProviderFailure> {
    match value["status"].as_str() {
        Some("confirmed") => Ok(LiveStatus::Confirmed),
        Some("tentative") => Ok(LiveStatus::Tentative),
        _ => Err(rejected()),
    }
}

fn checked(value: &Value, plan: &DeletePlan) -> Result<Event, ProviderFailure> {
    let (event, uid, organiser) = identity(value, plan.before())?;
    if uid != plan.uid() || organiser != plan.organiser() {
        return Err(rejected());
    }
    Ok(event)
}

impl GoogleCalendarProvider {
    fn restoration_url(&self, event: &Event) -> String {
        format!(
            "{}/calendars/{}/events/{}",
            self.base,
            urlencoding::encode(&event.source_id),
            urlencoding::encode(&event.id)
        )
    }

    async fn read_restoration(
        &self,
        token: &str,
        event: &Event,
    ) -> Result<Option<Value>, ProviderFailure> {
        let response = self
            .client
            .get(self.restoration_url(event))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| ProviderFailure::waiting("Could not inspect the Google event."))?;
        let status = response.status();
        if matches!(status.as_u16(), 404 | 410) {
            return Ok(None);
        }
        let body = bounded_text(response).await?;
        if !status.is_success() {
            return Err(response_failure(status, body));
        }
        serde_json::from_str(&body)
            .map(Some)
            .map_err(|_| ProviderFailure::waiting("Google returned an unreadable event."))
    }

    pub(super) async fn prepare_google_delete(
        &self,
        token: &str,
        event: &Event,
    ) -> Result<DeletePlan, ProviderFailure> {
        if !event.is_bounded()
            || !event.etag.as_deref().is_some_and(strong_etag)
            || event.remote_url.as_deref() != Some(event.id.as_str())
        {
            return Err(rejected());
        }
        let value = self
            .read_restoration(token, event)
            .await?
            .ok_or_else(rejected)?;
        let (current, uid, organiser) = identity(&value, event)?;
        if current != *event {
            return Err(ProviderFailure::rejected(
                "The event changed. Refresh before deleting it.",
            ));
        }
        DeletePlan::new(current, live_status(&value)?, uid, organiser)
    }

    async fn patch_status(
        &self,
        token: &str,
        plan: &DeletePlan,
        etag: &str,
        status: &str,
    ) -> Result<Value, ProviderFailure> {
        let response = self
            .client
            .patch(self.restoration_url(plan.before()))
            .bearer_auth(token)
            .header(reqwest::header::IF_MATCH, etag)
            .query(&[
                ("conferenceDataVersion", "1"),
                ("supportsAttachments", "true"),
            ])
            .json(&json!({"status": status}))
            .send()
            .await
            .map_err(|error| {
                if error.is_builder() {
                    ProviderFailure::rejected("Invalid Google event request.")
                } else {
                    ProviderFailure::uncertain(
                        "The event result is unknown. Check Google Calendar before continuing.",
                    )
                }
            })?;
        let code = response.status();
        let body = bounded_text(response).await.map_err(|_| {
            ProviderFailure::uncertain(
                "The event response could not be retained. Check Google Calendar.",
            )
        })?;
        if !code.is_success() {
            return Err(response_failure(code, body));
        }
        let value: Value = serde_json::from_str(&body).map_err(|_| {
            ProviderFailure::uncertain("Google returned an unreadable mutation receipt.")
        })?;
        let current = checked(&value, plan).map_err(|_| {
            ProviderFailure::uncertain("Google returned an incomplete mutation receipt.")
        })?;
        if value["status"].as_str() != Some(status) || current.etag.as_deref() == Some(etag) {
            return Err(ProviderFailure::uncertain(
                "Google returned an unexpected mutation version or status.",
            ));
        }
        Ok(value)
    }

    pub(super) async fn cancel_google_event(
        &self,
        token: &str,
        plan: &DeletePlan,
    ) -> Result<DeleteReceipt, ProviderFailure> {
        let etag = plan.before().etag.as_deref().ok_or_else(rejected)?;
        let value = self.patch_status(token, plan, etag, "cancelled").await?;
        let cancelled_etag = value["etag"].as_str().ok_or_else(|| {
            ProviderFailure::uncertain("Google returned no cancellation version.")
        })?;
        DeleteReceipt::acknowledged(plan.clone(), cancelled_etag.into())
    }

    pub(super) async fn restore_google_event(
        &self,
        token: &str,
        request: &RestoreRequest,
    ) -> Result<Event, ProviderFailure> {
        let receipt = request.receipt();
        let value = self
            .patch_status(
                token,
                receipt.plan(),
                receipt.cancelled_etag(),
                receipt.plan().status().as_str(),
            )
            .await?;
        checked(&value, receipt.plan()).map_err(|_| {
            ProviderFailure::uncertain("Google returned an incomplete restoration receipt.")
        })
    }

    pub(super) async fn inspect_google_deletion(
        &self,
        token: &str,
        plan: &DeletePlan,
    ) -> Result<Inspection, ProviderFailure> {
        let Some(value) = self.read_restoration(token, plan.before()).await? else {
            return Ok(Inspection::Missing);
        };
        let event = checked(&value, plan)?;
        if value["status"].as_str() == Some("cancelled") {
            return Ok(Inspection::Cancelled {
                etag: event.etag.ok_or_else(rejected)?,
            });
        }
        Ok(Inspection::Live {
            event,
            status: live_status(&value)?,
        })
    }
}
