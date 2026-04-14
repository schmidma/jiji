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

        if !repository.cache_root().exists() {
            create_dir_all(repository.cache_root()).wrap_err("failed to create cache directory")?;
        }

        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(repository.lock_path())
            .wrap_err("failed to create repository lock file")?;

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
        self.workspace_root().exists()
    }
}

#[cfg(test)]
mod test {
    use camino::Utf8Path;
    use tempfile::tempdir;

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
}
