// #![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::cargo)]

mod add;
mod cache;
mod configuration;
mod find_root;
mod gc;
mod hashing;
mod index;
mod init;
mod locking;
mod reference_file;
mod relative_path;
mod restore;
mod status;
mod storage;
mod with_added_extension;

pub use crate::configuration::{StorageListEntry, StorageListReport};
pub use crate::gc::GarbageCollectionReport;
pub use reference_file::{Reference, ReferenceFile};
pub use with_added_extension::WithAddedExtension;

use camino::{absolute_utf8, Utf8Component, Utf8Path, Utf8PathBuf};
use color_eyre::{eyre::Context as _, Result};
use std::env::current_dir;

use crate::locking::{LockMode, RepositoryLockGuard};
use crate::relative_path::AsRelativePath as _;

#[derive(Debug)]
pub struct JijiRepository {
    root: Utf8PathBuf,
}

impl JijiRepository {
    const WORKSPACE_DIR: &'static str = ".jiji";

    pub fn new(root: Utf8PathBuf) -> Result<Self> {
        Ok(Self { root })
    }

    pub fn workspace_root(&self) -> Utf8PathBuf {
        self.root.join(Self::WORKSPACE_DIR)
    }

    pub fn cache_root(&self) -> Utf8PathBuf {
        self.workspace_root().join("cache")
    }

    pub(crate) fn lock_path(&self) -> Utf8PathBuf {
        self.workspace_root().join(".lock")
    }

    pub(crate) fn repository_lock(&self) -> Result<crate::locking::RepositoryLock> {
        crate::locking::RepositoryLock::new(self.lock_path().as_std_path())
    }

    fn acquire_lock(&self, mode: LockMode, command: &'static str) -> Result<RepositoryLockGuard> {
        let lock_path = self.lock_path();
        self.repository_lock()?.acquire(mode, || {
            println!(
                "Waiting for repository lock at '{}' while running {}...",
                lock_path, command
            );
        })
    }

    pub(crate) fn read_lock(&self, command: &'static str) -> Result<RepositoryLockGuard> {
        self.acquire_lock(LockMode::Read, command)
    }

    pub(crate) fn write_lock(&self, command: &'static str) -> Result<RepositoryLockGuard> {
        self.acquire_lock(LockMode::Write, command)
    }

    /// Returns `path` relative to the repository root.
    /// Relative inputs are resolved from the current working directory.
    pub fn to_repo_relative_path(&self, path: impl AsRef<Utf8Path>) -> Result<Utf8PathBuf> {
        let path = path.as_ref();
        let working_directory = Utf8PathBuf::try_from(current_dir()?)
            .wrap_err("current directory is not valid utf-8")?;

        self.to_repo_relative_path_from(path, &working_directory)
    }

    /// Returns `path` relative to the repository root using `working_directory`
    /// as the base for relative inputs.
    pub fn to_repo_relative_path_from(
        &self,
        path: impl AsRef<Utf8Path>,
        working_directory: impl AsRef<Utf8Path>,
    ) -> Result<Utf8PathBuf> {
        let path = path.as_ref();
        let repo_root = self.absolute_root_from(working_directory.as_ref())?;
        let absolute_path = absolute_utf8_from(path, working_directory.as_ref())
            .wrap_err_with(|| format!("failed to canonicalize path: {path}"))?;

        let relative = absolute_path
            .strip_prefix(&repo_root)
            .wrap_err_with(|| format!("path {path} is not within repository root {}", self.root))?;

        Ok(relative.to_owned())
    }

    fn absolute_root_from(&self, working_directory: &Utf8Path) -> Result<Utf8PathBuf> {
        absolute_utf8_from(&self.root, working_directory).wrap_err_with(|| {
            format!(
                "failed to canonicalize repository root {} from {}",
                self.root, working_directory
            )
        })
    }

    pub(crate) fn to_user_facing_path(
        &self,
        repo_relative_path: impl AsRef<Utf8Path>,
    ) -> Result<Utf8PathBuf> {
        let absolute_path = absolute_utf8(self.root.join(repo_relative_path.as_ref()))
            .wrap_err("failed to canonicalize repository path")?;
        absolute_path
            .as_relative_path()
            .wrap_err("failed to format path relative to current working directory")
    }
}

fn absolute_utf8_from(path: &Utf8Path, working_directory: &Utf8Path) -> Result<Utf8PathBuf> {
    let absolute = if path.is_absolute() {
        absolute_utf8(path)?
    } else {
        absolute_utf8(working_directory.join(path))?
    };

    Ok(normalize_absolute_utf8(&absolute))
}

fn normalize_absolute_utf8(path: &Utf8Path) -> Utf8PathBuf {
    let mut normalized = Utf8PathBuf::new();
    for component in path.components() {
        match component {
            Utf8Component::Prefix(prefix) => normalized.push(prefix.as_str()),
            Utf8Component::RootDir => normalized.push(component.as_str()),
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir => {
                normalized.pop();
            }
            Utf8Component::Normal(component) => normalized.push(component),
        }
    }
    normalized
}

#[cfg(test)]
mod test_utils;

#[cfg(test)]
mod tests {
    use std::{fs, sync::mpsc, thread, time::Duration};

    use tempfile::tempdir;

    use crate::test_utils::{setup_repo, CurrentDirGuard};

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
    fn new_ignores_malformed_configuration_until_configuration_is_requested() -> Result<()> {
        let repo_dir = tempdir()?;
        let repo_root = <&Utf8Path>::try_from(repo_dir.path())?;
        fs::create_dir_all(repo_root.join(".jiji").as_std_path())?;
        fs::write(
            repo_root.join(".jiji/config.toml").as_std_path(),
            "not = [valid toml",
        )?;

        let repo = JijiRepository::new(repo_root.to_owned())?;

        assert_eq!(repo.workspace_root(), repo_root.join(".jiji"));

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

    #[test]
    fn relative_path_uses_current_working_directory_for_nested_file() -> Result<()> {
        let repo_dir = tempdir()?;
        let repo_root = <&Utf8Path>::try_from(repo_dir.path())?;
        let working_directory = repo_root.join("nested/deeper");
        std::fs::create_dir_all(working_directory.as_std_path())?;
        let _guard = CurrentDirGuard::set(&working_directory)?;

        let repo = JijiRepository::new(repo_root.to_owned())?;
        let relative = repo.to_repo_relative_path("file.txt")?;

        assert_eq!(relative, "nested/deeper/file.txt");

        Ok(())
    }

    #[test]
    fn relative_path_normalizes_parent_components_from_current_working_directory() -> Result<()> {
        let repo_dir = tempdir()?;
        let repo_root = <&Utf8Path>::try_from(repo_dir.path())?;
        let working_directory = repo_root.join("nested/deeper");
        std::fs::create_dir_all(working_directory.as_std_path())?;
        let _guard = CurrentDirGuard::set(&working_directory)?;

        let repo = JijiRepository::new(repo_root.to_owned())?;
        let relative = repo.to_repo_relative_path("../file.txt")?;

        assert_eq!(relative, "nested/file.txt");

        Ok(())
    }

    #[test]
    fn relative_path_does_not_preserve_repo_relative_looking_input() -> Result<()> {
        let repo_dir = tempdir()?;
        let repo_root = <&Utf8Path>::try_from(repo_dir.path())?;
        let working_directory = repo_root.join("nested");
        std::fs::create_dir_all(working_directory.as_std_path())?;
        let _guard = CurrentDirGuard::set(&working_directory)?;

        let repo = JijiRepository::new(repo_root.to_owned())?;
        let relative = repo.to_repo_relative_path("nested/file.txt")?;

        assert_eq!(relative, "nested/nested/file.txt");

        Ok(())
    }

    #[test]
    fn relative_path_from_nested_working_directory_is_anchored_to_that_directory() -> Result<()> {
        let repo_dir = tempdir()?;
        let repo_root = <&Utf8Path>::try_from(repo_dir.path())?;
        let repo = JijiRepository::new(repo_root.to_owned())?;
        let working_directory = repo_root.join("nested/deeper");

        let relative = repo.to_repo_relative_path_from("file.txt", &working_directory)?;

        assert_eq!(relative, "nested/deeper/file.txt");

        Ok(())
    }

    #[test]
    fn user_facing_path_from_nested_working_directory_is_relative_to_that_directory() -> Result<()>
    {
        let repo_dir = tempdir()?;
        let repo_root = <&Utf8Path>::try_from(repo_dir.path())?;
        let working_directory = repo_root.join("nested/deeper");
        std::fs::create_dir_all(working_directory.as_std_path())?;
        let _guard = CurrentDirGuard::set(&working_directory)?;

        let repo = JijiRepository::new(repo_root.to_owned())?;
        let path = repo.to_user_facing_path("nested/deeper/file.txt")?;

        assert_eq!(path, "file.txt");

        Ok(())
    }

    #[test]
    fn read_lock_returns_guard_that_blocks_writer_until_drop() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;
        let read_guard = repo.read_lock("test read")?;
        let (finished_tx, finished_rx) = mpsc::channel();

        thread::scope(|scope| {
            scope.spawn(|| {
                let writer_result = repo.write_lock("test write").map(|_guard| ());
                finished_tx
                    .send(writer_result)
                    .expect("writer result should send");
            });

            assert!(finished_rx.recv_timeout(Duration::from_millis(50)).is_err());

            drop(read_guard);

            finished_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("writer should finish after read guard is dropped")?;

            Ok::<_, color_eyre::Report>(())
        })?;

        Ok(())
    }
}
