use super::*;

#[derive(Clone, Copy)]
pub(super) enum Permission {
    Identity,
    Drive,
    CalendarRead,
    CalendarWrite,
}
impl Permission {
    fn permits(self, access: Access) -> bool {
        match self {
            Self::Identity => true,
            Self::Drive => access.drive,
            Self::CalendarRead => access.calendar_read,
            Self::CalendarWrite => access.calendar_read && access.calendar_write,
        }
    }
    fn message(self) -> &'static str {
        match self {
            Self::Identity => "Reconnect Google in Preferences.",
            Self::Drive => {
                "Google did not grant Drive app data access. Reconnect Google in Preferences and approve that permission."
            }
            Self::CalendarRead | Self::CalendarWrite => {
                "Approve the required Google Calendar permission in Preferences."
            }
        }
    }
}

pub(super) async fn access_token(
    state: &AppState,
    session: &Session,
    headers: &HeaderMap,
    permission: Permission,
) -> Result<Zeroizing<String>, (StatusCode, &'static str)> {
    let conflict = |message| (StatusCode::CONFLICT, message);
    let changed = || conflict("The Google connection changed. Use the current connection.");
    let key =
        session_key(headers).ok_or((StatusCode::UNAUTHORIZED, "Sign in before using Google."))?;
    let observed = state
        .profiles
        .grants
        .lock()
        .await
        .get(&key)
        .map(|grant| (grant.generation.clone(), grant.refresh_owner.clone()));
    let (generation, refresh_owner) =
        observed.ok_or_else(|| conflict("Connect Google in Preferences."))?;
    let _refresh = refresh_owner.lock().await;
    let sessions = state.sessions.lock().await;
    if !sessions.get(&key).is_some_and(|current| {
        current.identity.subject == session.identity.subject
            && current.created.elapsed() < Duration::from_secs(crate::SESSION_SECONDS)
    }) {
        return Err(changed());
    }
    let grants = state.profiles.grants.lock().await;
    let grant = grants
        .get(&key)
        .filter(|grant| grant.generation == generation)
        .ok_or_else(changed)?;
    if !permission.permits(grant.access) {
        return Err(conflict(permission.message()));
    }
    if grant.expires.saturating_duration_since(Instant::now()) >= REFRESH_MARGIN {
        return Ok(grant.access_token.clone());
    }
    let refresh = grant
        .refresh_token
        .clone()
        .ok_or_else(|| conflict("The Google connection expired. Reconnect in Preferences."))?;
    drop(grants);
    drop(sessions);
    let tokens = state.provider.refresh(&refresh).await.map_err(|_| {
        (
            StatusCode::BAD_GATEWAY,
            "Google did not renew the connection. Retry, or reconnect Google in Preferences.",
        )
    })?;
    if tokens
        .subject
        .as_deref()
        .is_some_and(|subject| subject != session.identity.subject)
    {
        return Err(changed());
    }
    // Session then grant matches the authentication middleware's lock order.
    let sessions = state.sessions.lock().await;
    if !sessions.get(&key).is_some_and(|current| {
        current.identity.subject == session.identity.subject
            && current.created.elapsed() < Duration::from_secs(crate::SESSION_SECONDS)
    }) {
        return Err(changed());
    }
    let mut grants = state.profiles.grants.lock().await;
    let grant = grants
        .get_mut(&key)
        .filter(|grant| grant.generation == generation)
        .ok_or_else(changed)?;
    grant.access = Access::from_scope(tokens.scope.as_deref(), grant.requested);
    grant.access_token = tokens.access_token;
    if let Some(token) = tokens.refresh_token {
        grant.refresh_token = Some(token);
    }
    grant.expires = Instant::now() + Duration::from_secs(tokens.expires_in.clamp(30, 86_400));
    if !permission.permits(grant.access) {
        return Err(conflict(permission.message()));
    }
    Ok(grant.access_token.clone())
}
