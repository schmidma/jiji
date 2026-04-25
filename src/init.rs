use std::fs::{create_dir_all, OpenOptions};

use camino::Utf8PathBuf;
use color_eyre::{eyre::Context as _, Result};
use tracing::info;

use crate::{configuration::Configuration, JijiRepository};

impl JijiRepository {
    /// Initialize a new repository at the given `repository_root`.
    /// Creates and populates the `.jiji/` directory with the necessary structure.
    pub fn init(path: impl Into<Utf8PathBuf>) -> Result<Self> {
        let repository = Self::new(path.into()).wrap_err("failed to create repository")?;

        if !repository.workspace_root().exists() {
            create_dir_all(repository.workspace_root())
                .wrap_err("failed to create workspace directory")?;
            info!(
                "initialized new repository at {root}",
                root = repository.root
            );
        }

        repository.ensure_lock_file()?;

        let _guard = repository.write_lock("init")?;
        repository.ensure_workspace_gitignore()?;

        if !repository.cache_root().exists() {
            create_dir_all(repository.cache_root()).wrap_err("failed to create cache directory")?;
        }

        let config_path = repository.workspace_root().join("config.toml");
        if !config_path.exists() {
            let default_config = Configuration::default();
            default_config
                .save(config_path)
                .wrap_err("failed to write default configuration file")?;
        }

        Ok(repository)
    }

    pub fn is_initialized(&self) -> bool {
        self.workspace_root().is_dir()
            && self.cache_root().is_dir()
            && self.lock_path().is_file()
            && self.workspace_root().join("config.toml").is_file()
    }

    #[doc(hidden)]
    pub fn ensure_initialized_or_migrate_lock(&self) -> Result<bool> {
        if self.is_initialized() {
            return Ok(true);
        }

        if self.is_legacy_initialized_without_lock() {
            self.ensure_lock_file()?;
            return Ok(true);
        }

        if self.lock_path().is_file() {
            let guard = self.read_lock("repository initialization")?;
            drop(guard);

            if self.is_initialized() {
                return Ok(true);
            }

            if self.is_legacy_initialized_without_lock() {
                self.ensure_lock_file()?;
                return Ok(true);
            }
        }

        Ok(false)
    }

    pub(crate) fn ensure_lock_file(&self) -> Result<()> {
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.lock_path())
            .wrap_err("failed to create repository lock file")?;

        Ok(())
    }

    fn is_legacy_initialized_without_lock(&self) -> bool {
        self.workspace_root().is_dir()
            && self.cache_root().is_dir()
            && !self.lock_path().exists()
            && self.workspace_root().join("config.toml").is_file()
    }
}

#[cfg(test)]
mod test {
    use std::{sync::mpsc, thread, time::Duration};

    use camino::Utf8Path;
    use tempfile::tempdir;

    use crate::locking::{LockMode, RepositoryLock};

    use super::*;

    #[test]
    fn init_creates_workspace_and_cache() -> Result<()> {
        let tmp = tempdir()?;
        let repo_path = <&Utf8Path>::try_from(tmp.path()).unwrap();

        let repo = JijiRepository::init(repo_path)?;

        assert!(repo.is_initialized(), "repo should be initialized");
        assert!(
            repo.workspace_root().exists(),
            ".jiji workspace should exist"
        );
        assert!(repo.cache_root().exists(), ".jiji/cache should exist");

        Ok(())
    }

    #[test]
    fn init_creates_repository_lock_file() -> Result<()> {
        let tmp = tempdir()?;
        let repo_path = <&Utf8Path>::try_from(tmp.path()).unwrap();

        let repo = JijiRepository::init(repo_path)?;

        assert!(
            repo.workspace_root().join(".lock").exists(),
            ".jiji/.lock should exist"
        );

        Ok(())
    }

    #[test]
    fn init_creates_workspace_gitignore_for_local_state() -> Result<()> {
        let tmp = tempdir()?;
        let repo_path = <&Utf8Path>::try_from(tmp.path()).unwrap();

        let repo = JijiRepository::init(repo_path)?;

        let gitignore = std::fs::read_to_string(repo.workspace_root().join(".gitignore"))?;
        assert_eq!(gitignore, "/cache/\n/.lock\n/config.local.toml\n");

        Ok(())
    }

    #[test]
    fn init_workspace_gitignore_is_idempotent() -> Result<()> {
        let tmp = tempdir()?;
        let repo_path = <&Utf8Path>::try_from(tmp.path()).unwrap();

        JijiRepository::init(repo_path)?;
        let repo = JijiRepository::init(repo_path)?;

        let gitignore = std::fs::read_to_string(repo.workspace_root().join(".gitignore"))?;
        assert_eq!(gitignore, "/cache/\n/.lock\n/config.local.toml\n");

        Ok(())
    }

    #[test]
    fn init_is_idempotent() -> Result<()> {
        let tmp = tempdir()?;
        let repo_path = <&Utf8Path>::try_from(tmp.path()).unwrap();

        // First init
        let repo1 = JijiRepository::init(repo_path)?;
        assert!(repo1.is_initialized());

        // Second init should not fail
        let repo2 = JijiRepository::init(repo_path)?;
        assert!(repo2.is_initialized());

        Ok(())
    }

    #[test]
    fn is_initialized_false_when_not_created() -> Result<()> {
        let tmp = tempdir()?;
        let repo_path = <&Utf8Path>::try_from(tmp.path()).unwrap();

        let repo = JijiRepository::new(repo_path.to_owned())?;

        assert!(
            !repo.is_initialized(),
            "empty repo should not be initialized"
        );

        Ok(())
    }

    #[test]
    fn is_initialized_false_when_workspace_is_incomplete() -> Result<()> {
        let tmp = tempdir()?;
        let repo_path = <&Utf8Path>::try_from(tmp.path()).unwrap();
        let repo = JijiRepository::new(repo_path.to_owned())?;

        create_dir_all(repo.workspace_root())?;

        assert!(
            !repo.is_initialized(),
            "workspace directory alone should not count as initialized"
        );

        Ok(())
    }

    #[test]
    fn init_waits_for_existing_repository_lock() -> Result<()> {
        let tmp = tempdir()?;
        let repo_path = <&Utf8Path>::try_from(tmp.path()).unwrap();
        let workspace = repo_path.join(".jiji");
        create_dir_all(&workspace)?;

        let lock_path = workspace.join(".lock");
        let lock = RepositoryLock::new(lock_path.as_std_path())?;
        let lock_guard = lock.acquire(LockMode::Write, || {})?;

        let (finished_tx, finished_rx) = mpsc::channel();

        thread::scope(|scope| {
            scope.spawn(|| {
                finished_tx
                    .send(JijiRepository::init(repo_path.to_owned()))
                    .expect("init result should send");
            });

            thread::sleep(Duration::from_millis(150));
            assert!(
                finished_rx.try_recv().is_err(),
                "init should wait while the repository lock is held"
            );

            drop(lock_guard);

            let initialized = finished_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("init should finish after lock release")?;
            assert!(initialized.is_initialized());

            Ok::<_, color_eyre::Report>(())
        })?;

        Ok(())
    }
}
