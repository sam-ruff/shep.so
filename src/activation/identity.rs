//! Which executable a process runs, so an installed update is never handed
//! to the process that ran before it.
use serde::{Deserialize, Serialize};
use std::{path::Path, time::UNIX_EPOCH};

/// Bounded and content-free: a path digest plus the file's own metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Identity {
    path: String,
    device: u64,
    inode: u64,
    size: u64,
    modified: (u64, u32),
}

impl Identity {
    pub(super) fn current() -> Option<Self> {
        let path = std::env::current_exe().ok()?;
        // The magic link still resolves after the binary is replaced or deleted.
        #[cfg(target_os = "linux")]
        let metadata = std::fs::metadata("/proc/self/exe").ok()?;
        #[cfg(not(target_os = "linux"))]
        let metadata = std::fs::metadata(&path).ok()?;
        Some(Self::from_parts(&path, &metadata))
    }

    #[cfg(test)]
    pub(super) fn of(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self::from_parts(path, &metadata))
    }

    /// Test builds pretend to be a different executable through this inode.
    #[cfg(feature = "test-support")]
    pub(super) fn spoofed(mut self, inode: u64) -> Self {
        self.inode = inode;
        self
    }

    fn from_parts(path: &Path, metadata: &std::fs::Metadata) -> Self {
        use sha2::{Digest, Sha256};
        #[cfg(unix)]
        let (device, inode) = {
            use std::os::unix::fs::MetadataExt;
            (metadata.dev(), metadata.ino())
        };
        #[cfg(not(unix))]
        let (device, inode) = (0, 0);
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|elapsed| (elapsed.as_secs(), elapsed.subsec_nanos()))
            .unwrap_or_default();
        Self {
            path: format!("{:x}", Sha256::digest(path.as_os_str().as_encoded_bytes())),
            device,
            inode,
            size: metadata.len(),
            modified,
        }
    }
}

/// How a launching binary relates to the one the owner published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Relation {
    /// The same executable, or one this launch cannot identify.
    Same,
    /// The owner published no identity, so it predates this protocol.
    Older,
    Different,
}

pub(super) fn relation(own: Option<&Identity>, published: Option<&Identity>) -> Relation {
    match (own, published) {
        (None, _) => Relation::Same,
        (Some(_), None) => Relation::Older,
        (Some(own), Some(published)) if own == published => Relation::Same,
        _ => Relation::Different,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn the_running_binary_identifies_itself_consistently() {
        let first = Identity::current().expect("test binary identity");
        let second = Identity::current().expect("test binary identity");
        assert_eq!(first, second);
        assert_eq!(relation(Some(&first), Some(&second)), Relation::Same);
        assert!(serde_json::to_vec(&first).expect("bounded record").len() < 256);
    }

    #[test]
    fn a_replaced_binary_has_a_different_identity() {
        let directory = tempfile::tempdir().expect("fixture directory");
        let path = directory.path().join("shep");
        std::fs::write(&path, b"first build").expect("first build");
        let before = Identity::of(&path).expect("first identity");
        let mut replacement = tempfile::NamedTempFile::new_in(directory.path()).expect("staged");
        replacement
            .write_all(b"second build, longer")
            .expect("second build");
        replacement.persist(&path).expect("atomic replace");
        let after = Identity::of(&path).expect("second identity");
        assert_ne!(before, after);
        assert_eq!(relation(Some(&after), Some(&before)), Relation::Different);
        assert_eq!(
            before.path, after.path,
            "the same path only differs in file"
        );
    }

    #[test]
    fn a_deleted_binary_cannot_be_identified_and_old_owners_are_older() {
        let directory = tempfile::tempdir().expect("fixture directory");
        let path = directory.path().join("shep");
        std::fs::write(&path, b"installed build").expect("build");
        let installed = Identity::of(&path).expect("identity");
        std::fs::remove_file(&path).expect("uninstall");
        assert!(Identity::of(&path).is_none());
        assert_eq!(relation(Some(&installed), None), Relation::Older);
        assert_eq!(relation(None, Some(&installed)), Relation::Same);
        assert_eq!(relation(None, None), Relation::Same);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn a_spoofed_inode_is_a_different_binary() {
        let own = Identity::current().expect("test binary identity");
        let spoofed = own.clone().spoofed(own.inode.wrapping_add(1));
        assert_eq!(relation(Some(&spoofed), Some(&own)), Relation::Different);
    }
}
