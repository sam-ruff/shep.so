use crate::model::GoogleAccess;

pub(super) fn access(scope: Option<&str>) -> GoogleAccess {
    let Some(scope) = scope else {
        return GoogleAccess::default();
    };
    let has = |name: &str| {
        scope
            .split_ascii_whitespace()
            .any(|s| s.strip_prefix("https://www.googleapis.com/auth/") == Some(name))
    };
    let list = has("calendar")
        || has("calendar.readonly")
        || has("calendar.calendarlist")
        || has("calendar.calendarlist.readonly");
    let write = has("calendar") || has("calendar.events");
    GoogleAccess {
        known: true,
        drive: has("drive.appdata"),
        calendar_read: list
            && (write || has("calendar.readonly") || has("calendar.events.readonly")),
        calendar_write: list && write,
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Service {
    Drive,
    ProfileSync,
    CalendarRead,
    CalendarWrite,
}
impl Service {
    pub(super) fn check(self, access: GoogleAccess) -> anyhow::Result<()> {
        let (allowed, name) = match self {
            Self::Drive => (access.drive_allowed(), "Drive backup"),
            Self::ProfileSync => (access.drive_allowed(), "Profile sync"),
            Self::CalendarRead => (access.calendar_allowed(), "Calendar sync"),
            Self::CalendarWrite => (access.calendar_write_allowed(), "Calendar editing"),
        };
        anyhow::ensure!(
            allowed,
            "Google did not grant {name} access. Reconnect Google in Preferences and approve that permission."
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_sync_requires_drive_permission_and_names_the_affected_service() {
        let denied = GoogleAccess {
            known: true,
            ..Default::default()
        };
        assert!(
            Service::ProfileSync
                .check(denied)
                .unwrap_err()
                .to_string()
                .contains("Profile sync")
        );
        assert!(
            Service::ProfileSync
                .check(GoogleAccess {
                    drive: true,
                    ..denied
                })
                .is_ok()
        );
    }
}
