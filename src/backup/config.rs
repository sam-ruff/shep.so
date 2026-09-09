//! Device-local backup destinations. Legacy scalar settings remain the selected editor.
use super::BackupTarget;
use crate::model::{BackupDestination, Preferences};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Destination {
    pub id: String,
    pub name: String,
    #[serde(default = "included_by_default")]
    pub included: bool,
    pub destination: BackupDestination,
    pub folder: String,
    #[serde(default)]
    pub s3: super::s3::Settings,
    #[serde(default)]
    pub sftp: super::sftp::Settings,
    #[serde(default)]
    pub ftp: super::ftp::Settings,
    #[serde(default)]
    pub format: super::format::Options,
    pub copies: usize,
    pub hours: u64,
    pub accounts: bool,
    pub automatic: bool,
    pub last_backup: Option<i64>,
    pub ready: bool,
}
fn included_by_default() -> bool {
    true
}

impl Destination {
    pub fn capture(prefs: &Preferences, id: String, name: String) -> Self {
        Self {
            id,
            name,
            included: true,
            destination: prefs.backup_destination,
            folder: prefs.backup_folder.clone(),
            s3: prefs.backup_s3.clone(),
            sftp: prefs.backup_sftp.clone(),
            ftp: prefs.backup_ftp.clone(),
            format: prefs.backup_format,
            copies: prefs.backup_copies,
            hours: prefs.backup_hours,
            accounts: prefs.backup_accounts,
            automatic: prefs.auto_backup,
            last_backup: prefs.last_backup,
            ready: prefs.backup_ready,
        }
    }
    pub fn apply(&self, prefs: &mut Preferences) {
        prefs.backup_destination = self.destination;
        prefs.backup_folder = self.folder.clone();
        prefs.backup_s3 = self.s3.clone();
        prefs.backup_sftp = self.sftp.clone();
        prefs.backup_ftp = self.ftp.clone();
        prefs.backup_format = self.format;
        prefs.backup_copies = self.copies;
        prefs.backup_hours = self.hours;
        prefs.backup_accounts = self.accounts;
        prefs.auto_backup = self.automatic;
        prefs.last_backup = self.last_backup;
        prefs.backup_ready = self.ready;
    }
    pub fn target(&self, prefs: &Preferences) -> BackupTarget {
        match self.destination {
            BackupDestination::Local => BackupTarget::Local(self.folder.clone()),
            BackupDestination::S3 => BackupTarget::S3(self.s3.identity()),
            BackupDestination::Sftp => BackupTarget::Sftp(self.sftp.identity()),
            BackupDestination::Ftp => BackupTarget::Ftp(self.ftp.identity()),
            BackupDestination::GoogleDrive => BackupTarget::GoogleDrive {
                client_id: prefs.active_google_client().to_owned(),
                connection_id: prefs.google_connection_id.clone(),
            },
        }
    }
}

/// Store the displayed form before changing selection or saving preferences.
pub fn capture_editor(prefs: &mut Preferences) {
    let Some(id) = prefs.backup_selected.clone() else {
        return;
    };
    if let Some(index) = prefs.backup_destinations.iter().position(|d| d.id == id) {
        let included = prefs.backup_destinations[index].included;
        let name = prefs.backup_destinations[index].name.clone();
        prefs.backup_destinations[index] = Destination::capture(prefs, id, name);
        prefs.backup_destinations[index].included = included;
    }
}

pub fn select(prefs: &mut Preferences, id: &str) -> anyhow::Result<()> {
    capture_editor(prefs);
    let destination = prefs
        .backup_destinations
        .iter()
        .find(|d| d.id == id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Choose an existing backup destination."))?;
    destination.apply(prefs);
    prefs.backup_selected = Some(destination.id);
    Ok(())
}

pub fn add(prefs: &mut Preferences) -> anyhow::Result<()> {
    anyhow::ensure!(
        prefs.backup_destinations.len() < 32,
        "Use at most 32 backup destinations."
    );
    if prefs.backup_destinations.is_empty() {
        let first = Destination::capture(
            prefs,
            uuid::Uuid::new_v4().to_string(),
            "Main backup".into(),
        );
        prefs.backup_selected = Some(first.id.clone());
        prefs.backup_destinations.push(first);
    }
    capture_editor(prefs);
    let mut next = Destination::capture(
        &Preferences::default(),
        uuid::Uuid::new_v4().to_string(),
        format!("Backup {}", prefs.backup_destinations.len() + 1),
    );
    next.folder.clear();
    next.apply(prefs);
    prefs.backup_selected = Some(next.id.clone());
    prefs.backup_destinations.push(next);
    Ok(())
}

pub fn remove_selected(prefs: &mut Preferences) -> anyhow::Result<()> {
    anyhow::ensure!(
        prefs.backup_destinations.len() > 1,
        "Keep at least one backup destination."
    );
    let id = prefs
        .backup_selected
        .take()
        .ok_or_else(|| anyhow::anyhow!("Choose a destination first."))?;
    prefs.backup_destinations.retain(|d| d.id != id);
    let first = prefs.backup_destinations[0].clone();
    first.apply(prefs);
    prefs.backup_selected = Some(first.id);
    Ok(())
}

pub fn validate(prefs: &Preferences) -> anyhow::Result<()> {
    anyhow::ensure!(
        prefs.backup_format.encrypted() || !prefs.backup_accounts,
        "Account passwords require an encrypted backup."
    );
    anyhow::ensure!(
        prefs.backup_destinations.len() <= 32,
        "Use at most 32 backup destinations."
    );
    if prefs.backup_destination == BackupDestination::S3 {
        prefs.backup_s3.validate_draft()?;
    }
    if prefs.backup_destination == BackupDestination::Sftp {
        prefs.backup_sftp.validate_draft()?;
    }
    if prefs.backup_destination == BackupDestination::Ftp {
        prefs.backup_ftp.validate_draft()?;
    }
    let mut ids = std::collections::HashSet::new();
    let mut targets = std::collections::HashSet::new();
    for d in &prefs.backup_destinations {
        anyhow::ensure!(
            d.format.encrypted() || !d.accounts,
            "Account passwords require an encrypted backup."
        );
        if d.destination == BackupDestination::S3 {
            d.s3.validate_draft()?;
        }
        if d.destination == BackupDestination::Sftp {
            d.sftp.validate_draft()?;
        }
        if d.destination == BackupDestination::Ftp {
            d.ftp.validate_draft()?;
        }
        let target = match d.destination {
            BackupDestination::Local if d.folder.trim().is_empty() => None,
            BackupDestination::Local => {
                Some(format!("local:{}", lexical_path(&d.folder).display()))
            }
            BackupDestination::GoogleDrive => Some("google-drive".to_string()),
            BackupDestination::Sftp if d.sftp.host.is_empty() || d.sftp.directory.is_empty() => {
                None
            }
            BackupDestination::Sftp => {
                let id = d.sftp.identity();
                Some(format!(
                    "sftp:{}",
                    serde_json::to_string(&(id.host, id.port, id.directory))?
                ))
            }
            BackupDestination::Ftp if d.ftp.host.is_empty() || d.ftp.directory.is_empty() => None,
            BackupDestination::Ftp => {
                let id = d.ftp.identity();
                Some(format!(
                    "ftp:{}",
                    serde_json::to_string(&(id.host, id.port, id.directory))?
                ))
            }
            BackupDestination::S3 if d.s3.bucket.is_empty() => None,
            BackupDestination::S3 => {
                Some(format!("s3:{}", serde_json::to_string(&d.s3.identity())?))
            }
        };
        anyhow::ensure!(
            target.is_none_or(|target| targets.insert(target)),
            "This backup destination is already configured. Choose a different folder or edit the existing destination."
        );
        anyhow::ensure!(
            uuid::Uuid::parse_str(&d.id).is_ok() && ids.insert(&d.id),
            "Backup destination identifiers must be unique."
        );
        anyhow::ensure!(
            !d.name.trim().is_empty() && d.name.chars().count() <= 60,
            "Give each backup destination a name of 1–60 characters."
        );
        anyhow::ensure!(
            (1..=100).contains(&d.copies) && (1..=8760).contains(&d.hours),
            "Choose 1–100 copies and an interval of 1–8760 hours."
        );
    }
    anyhow::ensure!(
        prefs
            .backup_selected
            .as_ref()
            .is_none_or(|id| ids.contains(id)),
        "The selected backup destination no longer exists."
    );
    Ok(())
}

pub fn configurations(prefs: &Preferences) -> Vec<Preferences> {
    if prefs.backup_destinations.is_empty() {
        return vec![prefs.clone()];
    }
    let mut current = prefs.clone();
    capture_editor(&mut current);
    current
        .backup_destinations
        .iter()
        .map(|d| {
            let mut configured = current.clone();
            d.apply(&mut configured);
            configured.backup_selected = Some(d.id.clone());
            configured
        })
        .collect()
}

pub fn resolve(prefs: &Preferences, target: &BackupTarget) -> anyhow::Result<Preferences> {
    configurations(prefs)
        .into_iter()
        .find(|p| BackupTarget::from_preferences(p) == *target)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "The backup destination changed. Choose its current settings and try again."
            )
        })
}

/// Preserve provider-owned history while accepting unrelated edits to every destination.
pub fn preserve_metadata(previous: &Preferences, requested: &mut Preferences) {
    capture_editor(requested);
    let old = configurations(previous);
    let google_available =
        !requested.google_lifecycle.disconnected && requested.google_grant.access.drive_allowed();
    let current_google = requested.clone();
    for destination in &mut requested.backup_destinations {
        let target = destination.target(&current_google);
        let saved = old
            .iter()
            .find(|p| BackupTarget::from_preferences(p) == target);
        destination.last_backup = saved.and_then(|p| p.last_backup);
        destination.ready =
            saved.is_some_and(|p| p.backup_ready && p.backup_format == destination.format);
        if destination.destination == BackupDestination::GoogleDrive && !google_available {
            destination.automatic = false;
            destination.ready = false;
        }
    }
    if let Some(id) = &requested.backup_selected
        && let Some(selected) = requested
            .backup_destinations
            .iter()
            .find(|d| &d.id == id)
            .cloned()
    {
        selected.apply(requested);
    }
}

pub fn record(prefs: &mut Preferences, target: &BackupTarget, time: i64, ready: bool) {
    let context = prefs.clone();
    for destination in &mut prefs.backup_destinations {
        if destination.target(&context) == *target {
            destination.last_backup = Some(time);
            destination.ready = ready;
        }
    }
    if BackupTarget::from_preferences(prefs) == *target {
        prefs.last_backup = Some(time);
        prefs.backup_ready = ready;
    }
}

/// A late copy acknowledges its actual format, never newly edited settings.
pub fn record_format(
    prefs: &mut Preferences,
    target: &BackupTarget,
    format: super::format::Options,
    time: i64,
    ready: bool,
) {
    let context = prefs.clone();
    for destination in &mut prefs.backup_destinations {
        if destination.target(&context) == *target {
            destination.last_backup = Some(time);
            if destination.format == format {
                destination.ready = ready;
            }
        }
    }
    if BackupTarget::from_preferences(prefs) == *target {
        prefs.last_backup = Some(time);
        if prefs.backup_format == format {
            prefs.backup_ready = ready;
        }
    }
}

pub fn pause(prefs: &mut Preferences, target: &BackupTarget) {
    let context = prefs.clone();
    for destination in &mut prefs.backup_destinations {
        if destination.target(&context) == *target {
            destination.ready = false;
        }
    }
    if BackupTarget::from_preferences(prefs) == *target {
        prefs.backup_ready = false;
    }
}

fn lexical_path(path: &str) -> std::path::PathBuf {
    let mut normalized = std::path::PathBuf::new();
    for component in std::path::Path::new(path).components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

/// Filesystem identity validation belongs on the background storage worker.
pub(crate) fn locations_changed(previous: &Preferences, next: &Preferences) -> bool {
    let locations = |prefs: &Preferences| {
        prefs
            .backup_destinations
            .iter()
            .map(|d| (d.destination, d.folder.clone()))
            .collect::<Vec<_>>()
    };
    locations(previous) != locations(next)
}

pub(crate) fn validate_filesystem(prefs: &Preferences) -> anyhow::Result<()> {
    let mut paths = Vec::new();
    for destination in &prefs.backup_destinations {
        if destination.destination != BackupDestination::Local
            || destination.folder.trim().is_empty()
        {
            continue;
        }
        let path = std::path::PathBuf::from(&destination.folder);
        if !path.is_absolute() {
            continue;
        }
        let mut existing = path.clone();
        let mut suffix = Vec::new();
        while !existing.try_exists()? {
            if let Some(name) = existing.file_name() {
                suffix.push(name.to_owned());
            }
            anyhow::ensure!(existing.pop(), "Choose a valid absolute backup folder.");
        }
        let mut canonical = existing.canonicalize()?;
        for name in suffix.iter().rev() {
            canonical.push(name);
        }
        anyhow::ensure!(
            !paths.contains(&canonical),
            "Two backup destinations point to the same folder. Edit the existing destination instead."
        );
        if canonical.try_exists()? {
            for previous in &paths {
                anyhow::ensure!(
                    !previous.try_exists()? || !crate::transfer::same_file(previous, &canonical)?,
                    "Two backup destinations refer to the same folder. Edit the existing destination instead."
                );
            }
        }
        paths.push(canonical);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiple_backup_migration_selection_and_removal_keep_the_other_destination() {
        let mut prefs = Preferences {
            backup_folder: "/original".into(),
            auto_backup: true,
            last_backup: Some(42),
            backup_ready: true,
            ..Default::default()
        };
        add(&mut prefs).unwrap();
        let first = prefs.backup_destinations[0].clone();
        assert_eq!(first.folder, "/original");
        assert!(first.automatic && first.ready);
        assert_eq!(first.last_backup, Some(42));
        prefs.backup_folder = "/second".into();
        prefs.backup_copies = 13;
        let second = prefs.backup_selected.clone().unwrap();
        select(&mut prefs, &first.id).unwrap();
        assert_eq!(prefs.backup_folder, "/original");
        assert_eq!(prefs.last_backup, Some(42));
        select(&mut prefs, &second).unwrap();
        assert_eq!(prefs.backup_folder, "/second");
        assert_eq!(prefs.backup_copies, 13);
        remove_selected(&mut prefs).unwrap();
        assert_eq!(prefs.backup_selected, Some(first.id));
        assert_eq!(prefs.backup_folder, "/original");
        assert!(remove_selected(&mut prefs).is_err());
    }
}
