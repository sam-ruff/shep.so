//! The Preferences Sign in with Google action: this build's client, the
//! saved-settings handshake before OAuth starts, and cancelling a browser wait.
use super::*;
use crate::providers::google::{CancellationToken, client};

impl App {
    pub(super) fn google_sign_in_label(&self) -> &'static str {
        if self.google_connected {
            "Reconnect Google"
        } else {
            "Sign in with Google"
        }
    }

    pub(super) fn google_sign_in_enabled(&self) -> bool {
        self.google_client.is_some()
            && !self.busy.contains("google")
            && !self.busy.contains("google-disconnect")
            && !self.preferences.google_lifecycle.cleanup_pending
            && self.preferences.requested_google_services().any()
    }

    /// A sign-in started here is still running and has not been cancelled.
    pub(super) fn google_waiting(&self) -> bool {
        self.google_sign_in
            .as_ref()
            .is_some_and(|token| !token.is_cancelled())
    }

    /// The working connection came from a self-configured client.
    pub(super) fn google_legacy_client(&self) -> bool {
        self.google_connected
            && client::legacy_grant(&self.preferences, self.google_client.as_deref())
    }

    /// Save the displayed setup first; OAuth starts once that save is acknowledged.
    pub(super) fn request_google_login(&mut self, retry: bool) {
        if self.google_client.is_none() {
            self.notice(client::NOT_CONFIGURED, true);
            return;
        }
        if let Err(e) = self.read_preferences() {
            self.notice(e.to_string(), true);
            return;
        }
        if !self.preferences.requested_google_services().any() {
            self.notice(
                "Choose Drive backup or Calendar access before signing in.",
                true,
            );
            return;
        }
        self.preferences.google_services = Some(self.preferences.requested_google_services());
        let request = self.preference_sync.changed();
        self.pending_google_login = Some((request, self.preferences.clone(), retry));
        if !self.queue_preference_write(request, self.preferences.clone()) {
            self.pending_google_login = None;
        }
    }

    /// Runs with the acknowledged settings; later edits need another sign-in.
    pub(super) fn start_google_login(&mut self, prefs: Preferences, retry: bool) {
        if prefs.google_services != self.preferences.google_services
            || prefs.google_lifecycle.revision != self.preferences.google_lifecycle.revision
        {
            self.notice(
                "Google setup changed. Sign in again with the current permissions.",
                true,
            );
            return;
        }
        let cancel = CancellationToken::new();
        if self.try_command(Command::GoogleLogin(prefs, retry, cancel.clone())) {
            self.google_sign_in = Some(cancel);
        }
    }

    pub(super) fn cancel_google_sign_in(&mut self) {
        if let Some(token) = &self.google_sign_in {
            token.cancel();
        }
    }

    pub(super) fn google_sign_in_state(&self) -> serde_json::Value {
        serde_json::json!({
            "available": self.google_client.is_some(),
            "label": self.google_sign_in_label(),
            "enabled": self.google_sign_in_enabled(),
            "waiting": self.google_waiting(),
            "legacy_client": self.google_legacy_client(),
        })
    }
}
