// #![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::cargo)]

mod add;
mod cache;
mod configuration;
mod find_root;
mod hashing;
mod index;
mod init;
mod reference_file;
mod relative_path;
mod restore;
mod status;
mod storage;
mod with_added_extension;

use camino::{absolute_utf8, Utf8Path, Utf8PathBuf};
use color_eyre::{eyre::Context as _, Result};

use crate::configuration::Configuration;

#[derive(Debug)]
pub struct JijiRepository {
    root: Utf8PathBuf,
    configuration: Configuration,
}

impl JijiRepository {
    const WORKSPACE_DIR: &'static str = ".jiji";

    pub fn new(root: Utf8PathBuf) -> Result<Self> {
        let configuration = Configuration::load(root.join(Self::WORKSPACE_DIR).join("config.toml"))
            .wrap_err("failed to load repository configuration")?;
        Ok(Self {
            root,
            configuration,
        })
    }

    pub fn workspace_root(&self) -> Utf8PathBuf {
        self.root.join(Self::WORKSPACE_DIR)
    }

    pub fn cache_root(&self) -> Utf8PathBuf {
        self.workspace_root().join("cache")
    }

    pub fn config(&self) -> &Configuration {
        &self.configuration
    }

    /// Returns the repository-relative path for a given path.
    /// Ensures the path is absolute and located inside the repository root.
    pub fn to_repo_relative_path(&self, path: impl AsRef<Utf8Path>) -> Result<Utf8PathBuf> {
        let path = path.as_ref();

        let absolute_path =
            absolute_utf8(path).wrap_err_with(|| format!("failed to canonicalize path: {path}"))?;

        let repo_root = absolute_utf8(&self.root)
            .wrap_err_with(|| format!("failed to canonicalize repository root: {}", self.root))?;

        let relative = absolute_path
            .strip_prefix(&repo_root)
            .wrap_err_with(|| format!("path {path} is not within repository root {}", self.root))?;

        Ok(relative.to_owned())
    }
}

#[cfg(test)]
mod test_utils;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_for_nested_file() -> Result<()> {
        let repo = JijiRepository::new("/home/user/project".into())?;

        let path = "foo/bar/data.txt";
        let relative = repo.to_repo_relative_path(repo.root.join(path))?;

        assert_eq!(
            relative, path,
            "nested file should resolve with its full relative path"
        );

        Ok(())
    }

    #[test]
    fn relative_path_error_if_path_outside_repo() -> Result<()> {
        let repo = JijiRepository::new("/home/user/project".into())?;

        let outside = "/somewhere/outside/the/repository.txt";
        let result = repo.to_repo_relative_path(outside);

        assert!(
            result.is_err(),
            "paths outside repository root must return an error"
        );

        Ok(())
    }

    #[test]
    fn relative_path_for_absolute_path() -> Result<()> {
        let repo = JijiRepository::new("/home/user/project".into())?;

        let file_path = "foo.txt";
        let absolute = repo.root.join(file_path);

        let relative = repo.to_repo_relative_path(absolute)?;

        assert_eq!(
            relative, file_path,
            "absolute paths inside repo should resolve to relative paths"
        );

        Ok(())
    }

    #[test]
    fn relative_path_for_relative_root() -> Result<()> {
        let repo = JijiRepository::new("../../root".into())?;

        let file_path = "foo/bar/data.txt";
        let path = repo.root.join(file_path);

        let relative = repo.to_repo_relative_path(path)?;

        assert_eq!(
            relative, file_path,
            "paths should be relative to the repository root"
        );

        Ok(())
    }
}
