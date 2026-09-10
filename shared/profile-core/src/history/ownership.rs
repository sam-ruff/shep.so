//! End an acquired OS file lock at its owner boundary, including duplicated
//! descriptors temporarily inherited by an unrelated child process.
use std::fs::File;

pub(crate) struct OwnedLock(File);

impl OwnedLock {
    /// Call only after acquiring the lock. Store this after the connection so
    /// connection teardown finishes before the ownership acknowledgement.
    pub(crate) fn acquired(file: File) -> Self {
        Self(file)
    }

    #[cfg(all(test, unix))]
    pub(crate) fn duplicate(&self) -> File {
        self.0.try_clone().unwrap()
    }
}

impl Drop for OwnedLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Write};
    use std::process::{Child, Command, Stdio};

    struct HeldChild(Child);
    impl HeldChild {
        fn inherited(file: File) -> Self {
            let child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "history::ownership::tests::inherited_descriptor_reproduces_close_only_delay_and_explicit_unlock_releases_it", "--nocapture"])
                .env("SHEP_LOCK_INHERITANCE_FIXTURE", "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                // The child holds the duplicated description but never writes
                // to it. Stdout and stdin provide independent handshakes.
                .stderr(Stdio::from(file))
                .spawn().unwrap();
            let mut held = Self(child);
            let mut output = std::io::BufReader::new(held.0.stdout.take().unwrap());
            loop {
                let mut line = String::new();
                assert_ne!(output.read_line(&mut line).unwrap(), 0);
                if line.contains("inherited-lock-ready") {
                    break;
                }
            }
            held.0.stdout = Some(output.into_inner());
            held
        }
        fn finish(mut self) {
            self.0.stdin.take().unwrap().write_all(b"x").unwrap();
            assert!(self.0.wait().unwrap().success());
        }
    }
    impl Drop for HeldChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn inherited_descriptor_reproduces_close_only_delay_and_explicit_unlock_releases_it() {
        if std::env::var_os("SHEP_LOCK_INHERITANCE_FIXTURE").is_some() {
            println!("inherited-lock-ready");
            std::io::stdout().flush().unwrap();
            std::io::stdin().read_exact(&mut [0u8]).unwrap();
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("owner.lock");
        let open = || {
            std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&path)
                .unwrap()
        };
        let file = open();
        fs2::FileExt::try_lock_exclusive(&file).unwrap();
        let child = HeldChild::inherited(file.try_clone().unwrap());
        drop(file);
        let contender = open();
        let error = fs2::FileExt::try_lock_exclusive(&contender).unwrap_err();
        assert_eq!(
            error.raw_os_error(),
            fs2::lock_contended_error().raw_os_error()
        );
        child.finish();
        fs2::FileExt::try_lock_exclusive(&contender).unwrap();
        let owner = OwnedLock::acquired(contender);
        let child = HeldChild::inherited(owner.duplicate());
        drop(owner);
        let contender = open();
        fs2::FileExt::try_lock_exclusive(&contender).unwrap();
        fs2::FileExt::unlock(&contender).unwrap();
        child.finish();
        assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
    }
}
