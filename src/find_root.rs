use camino::{absolute_utf8, Utf8Path, Utf8PathBuf};
use color_eyre::eyre::{eyre, Context as _, Result};
use pathdiff::diff_utf8_paths;
use std::{
    fs, thread,
    time::{Duration, Instant},
};
use tracing::debug;

use crate::JijiRepository;

const INCOMPLETE_WORKSPACE_WAIT: Duration = Duration::from_millis(250);
const INCOMPLETE_WORKSPACE_POLL: Duration = Duration::from_millis(10);

impl JijiRepository {
    /// Search for a `.jiji` directory in the ancestors of `start` and return the path to the directory
    /// containing it.
    pub fn find_upwards_from(start: impl AsRef<Utf8Path>) -> Result<Self> {
        let start = absolute_utf8(start.as_ref()).wrap_err("failed to get absolute path")?;

        let root = find_repository_root(&start)?;

        let mut relative_root = diff_utf8_paths(root, &start).expect("paths have common prefix");
        if relative_root.as_str().is_empty() {
            relative_root = Utf8PathBuf::from(".");
        }

        Self::new(relative_root).wrap_err("failed to create repository")
    }
}

fn find_repository_root(start: &Utf8Path) -> Result<&Utf8Path> {
    for ancestor in start.ancestors() {
        if !has_jiji_workspace(ancestor) {
            continue;
        }

        let repository = JijiRepository::new(ancestor.to_owned())
            .wrap_err("failed to create repository while searching ancestors")?;
        if repository.wait_until_initialized_or_migrate_lock(INCOMPLETE_WORKSPACE_WAIT)? {
            return Ok(ancestor);
        }

        return Err(eyre!(
            "found incomplete repository workspace at {}",
            repository.workspace_root()
        ));
    }

    Err(eyre!(
        "searched all ancestors but could not find repository root"
    ))
}

impl JijiRepository {
    fn wait_until_initialized_or_migrate_lock(&self, timeout: Duration) -> Result<bool> {
        let deadline = Instant::now() + timeout;

        loop {
            if self.ensure_initialized_or_migrate_lock()? {
                return Ok(true);
            }

            if Instant::now() >= deadline {
                return Ok(false);
            }

            thread::sleep(INCOMPLETE_WORKSPACE_POLL);
        }
    }
}

fn has_jiji_workspace(ancestor: &Utf8Path) -> bool {
    let candidate = ancestor.join(".jiji");
    match fs::metadata(candidate.as_std_path()) {
        Ok(metadata) => metadata.is_dir(),
        Err(error) => {
            debug!("failed to get metadata for {candidate}: {error:#}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::mpsc, thread, time::Duration};

    use tempfile::tempdir;

    use crate::locking::{LockMode, RepositoryLock};

    use super::*;

    #[test]
    fn find_root_in_current_dir() -> Result<()> {
        let tmp = tempdir()?;
        let root = <&Utf8Path>::try_from(tmp.path())?;
        JijiRepository::init(root.to_owned())?;

        let repo = JijiRepository::find_upwards_from(root)?;

        assert_eq!(repo.root, ".", "root should be '.' for current dir");

        Ok(())
    }

    #[test]
    fn find_root_in_parent_dir() -> Result<()> {
        let tmp = tempdir()?;
        let root = <&Utf8Path>::try_from(tmp.path())?;
        JijiRepository::init(root.to_owned())?;

        let nested = root.join("a/b/c");
        fs::create_dir_all(&nested)?;

        let repo = JijiRepository::find_upwards_from(&nested)?;

        assert_eq!(
            repo.root, "../../..",
            "root should be relative path to start"
        );

        Ok(())
    }

    #[test]
    fn find_root_no_jiji_error() -> Result<()> {
        let tmp = tempdir()?;
        let root = <&Utf8Path>::try_from(tmp.path())?;

        let res = JijiRepository::find_upwards_from(root);

        assert!(
            res.is_err(),
            "should error if no .jiji directory is present in ancestors"
        );

        Ok(())
    }

    #[test]
    fn find_root_rejects_incomplete_workspace() -> Result<()> {
        let tmp = tempdir()?;
        let root = <&Utf8Path>::try_from(tmp.path())?;
        fs::create_dir_all(root.join(".jiji"))?;

        let res = JijiRepository::find_upwards_from(root);

        assert!(
            res.is_err(),
            "should error if .jiji exists without complete repository layout"
        );

        Ok(())
    }

    #[test]
    fn find_root_stops_at_incomplete_nested_workspace() -> Result<()> {
        let tmp = tempdir()?;
        let parent = <&Utf8Path>::try_from(tmp.path())?;
        JijiRepository::init(parent.to_owned())?;

        let nested = parent.join("nested/project");
        fs::create_dir_all(nested.join(".jiji"))?;

        let res = JijiRepository::find_upwards_from(&nested);

        assert!(
            res.is_err(),
            "should error at incomplete nested workspace instead of resolving to parent"
        );

        Ok(())
    }

    #[test]
    fn find_root_creates_lock_for_legacy_repository() -> Result<()> {
        let tmp = tempdir()?;
        let root = <&Utf8Path>::try_from(tmp.path())?;
        let workspace = root.join(".jiji");
        fs::create_dir_all(workspace.join("cache"))?;
        fs::write(workspace.join("config.toml"), "")?;

        let repo = JijiRepository::find_upwards_from(root)?;

        assert_eq!(repo.root, ".", "root should be '.' for current dir");
        assert!(
            workspace.join(".lock").is_file(),
            "legacy repository should receive a lock file during discovery"
        );

        Ok(())
    }

    #[test]
    fn find_root_waits_for_in_progress_init() -> Result<()> {
        let tmp = tempdir()?;
        let root = <&Utf8Path>::try_from(tmp.path())?;
        let workspace = root.join(".jiji");
        fs::create_dir_all(&workspace)?;

        let lock_path = workspace.join(".lock");
        let lock = RepositoryLock::new(lock_path.as_std_path())?;
        let lock_guard = lock.acquire(LockMode::Write, || {})?;

        let (finished_tx, finished_rx) = mpsc::channel();

        thread::scope(|scope| {
            scope.spawn(|| {
                finished_tx
                    .send(JijiRepository::find_upwards_from(root))
                    .expect("discovery result should send");
            });

            thread::sleep(Duration::from_millis(150));
            assert!(
                finished_rx.try_recv().is_err(),
                "discovery should wait while init holds the repository lock"
            );

            fs::create_dir_all(workspace.join("cache"))?;
            fs::write(workspace.join("config.toml"), "")?;
            drop(lock_guard);

            let repo = finished_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("discovery should finish after init releases lock")?;
            assert_eq!(repo.root, ".");

            Ok::<_, color_eyre::Report>(())
        })?;

        Ok(())
    }

    #[test]
    fn find_root_waits_for_init_bootstrap_before_lock_exists() -> Result<()> {
        let tmp = tempdir()?;
        let root = <&Utf8Path>::try_from(tmp.path())?;
        let workspace = root.join(".jiji");
        fs::create_dir_all(&workspace)?;

        let (finished_tx, finished_rx) = mpsc::channel();

        thread::scope(|scope| {
            scope.spawn(|| {
                finished_tx
                    .send(JijiRepository::find_upwards_from(root))
                    .expect("discovery result should send");
            });

            thread::sleep(Duration::from_millis(50));
            assert!(
                finished_rx.try_recv().is_err(),
                "discovery should wait briefly for init to create the repository lock"
            );

            let lock_path = workspace.join(".lock");
            let lock = RepositoryLock::new(lock_path.as_std_path())?;
            let lock_guard = lock.acquire(LockMode::Write, || {})?;

            thread::sleep(Duration::from_millis(50));
            assert!(
                finished_rx.try_recv().is_err(),
                "discovery should wait while init holds the repository lock"
            );

            fs::create_dir_all(workspace.join("cache"))?;
            fs::write(workspace.join("config.toml"), "")?;
            drop(lock_guard);

            let repo = finished_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("discovery should finish after init releases lock")?;
            assert_eq!(repo.root, ".");

            Ok::<_, color_eyre::Report>(())
        })?;

        Ok(())
    }
}
