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

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn descriptor(&self) -> std::os::fd::RawFd {
        std::os::fd::AsRawFd::as_raw_fd(&self.0)
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

#[cfg(all(test, target_os = "linux"))]
mod inheritance_tests {
    use crate::history::{Binding, Error, Journal};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::{Command, Stdio};

    const VARIABLE: &str = "SHEP_JOURNAL_INHERITANCE_FIXTURE";
    const O_CLOEXEC: u64 = 0o2000000;

    fn binding(profile: uuid::Uuid, generation: uuid::Uuid) -> Binding {
        Binding {
            namespace: "so.shep.fixture".into(),
            principal: "drive:fixture".into(),
            profile,
            generation,
        }
    }

    fn descriptors_naming(path: &std::path::Path) -> Vec<String> {
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .flatten()
            .filter_map(|entry| std::fs::read_link(entry.path()).ok())
            .filter(|target| target == path)
            .map(|target| target.display().to_string())
            .collect()
    }

    /// The child is exec'd while the parent holds the journal lock. It must
    /// not receive the descriptor, must see the journal as owned, and must
    /// be able to claim it once the parent drops its Journal while the child
    /// is still alive.
    #[test]
    fn journal_lock_is_close_on_exec_and_a_live_child_never_inherits_it() {
        if let Some(fixture) = std::env::var_os(VARIABLE) {
            let fixture = fixture.to_string_lossy().into_owned();
            let (path, ids) = fixture.split_once('\n').unwrap();
            let (profile, generation) = ids.split_once('\n').unwrap();
            let path = std::path::PathBuf::from(path);
            let binding = binding(profile.parse().unwrap(), generation.parse().unwrap());
            let mut lock_path = path.as_os_str().to_owned();
            lock_path.push(".history-lock");
            assert!(descriptors_naming(std::path::Path::new(&lock_path)).is_empty());
            assert!(matches!(
                Journal::open(&path, binding.clone()),
                Err(Error::Owned)
            ));
            println!("child-ready");
            std::io::stdout().flush().unwrap();
            std::io::stdin().read_exact(&mut [0u8]).unwrap();
            let reopened = Journal::open(&path, binding).unwrap();
            assert_eq!(reopened.state().unwrap().operations, 0);
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite");
        let profile = uuid::Uuid::new_v4();
        let generation = uuid::Uuid::new_v4();
        let journal = Journal::open(&path, binding(profile, generation)).unwrap();
        let descriptor = journal._lock.as_ref().unwrap().descriptor();
        let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{descriptor}")).unwrap();
        let flags = info
            .lines()
            .find_map(|line| line.strip_prefix("flags:"))
            .map(|value| u64::from_str_radix(value.trim(), 8).unwrap())
            .unwrap();
        assert_ne!(
            flags & O_CLOEXEC,
            0,
            "the lock descriptor must close on exec"
        );
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "history::ownership::inheritance_tests::journal_lock_is_close_on_exec_and_a_live_child_never_inherits_it",
                "--nocapture",
            ])
            .env(
                VARIABLE,
                format!("{}\n{profile}\n{generation}", path.canonicalize().unwrap().display()),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            assert_ne!(
                output.read_line(&mut line).unwrap(),
                0,
                "child exited early"
            );
            if line.contains("child-ready") {
                break;
            }
        }
        drop(journal);
        child.stdin.take().unwrap().write_all(b"x").unwrap();
        let status = child.wait().unwrap();
        let mut rest = String::new();
        output.read_to_string(&mut rest).unwrap();
        assert!(status.success(), "{rest}");
        assert!(rest.contains("1 passed"), "{rest}");
    }
}
