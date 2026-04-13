use std::{
    fmt::Debug,
    fs::{copy, create_dir_all},
};

use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    eyre::{bail, Context as _},
    Result,
};
use tracing::{debug, warn};

use crate::{
    index::{Directory, DirectoryChildren, File, Index},
    JijiRepository,
};

impl JijiRepository {
    /// Restores all repository entries whose paths lie under one of the given selected paths.
    ///
    /// For example, restoring `foo/bar` will restore `foo/bar/data.txt` but not `foo/bar_asdf`.
    pub fn restore(&self, paths: &[impl AsRef<Utf8Path> + Debug]) -> Result<()> {
        let repo_relative_paths: Vec<Utf8PathBuf> = paths
            .iter()
            .map(|path| self.to_repo_relative_path(path))
            .collect::<Result<_>>()?;

        let index = self.index().wrap_err("failed to index repository")?;

        for path in &repo_relative_paths {
            if !index_contains_path(&index, path) {
                bail!("path '{}' is not tracked", self.to_user_facing_path(path)?);
            }
        }

        for node in index.iter_nodes() {
            for file in &node.files {
                self.restore_file(file, &repo_relative_paths, &node.base)
                    .wrap_err_with(|| {
                        format!(
                            "failed to restore file '{}'",
                            self.to_user_facing_path(node.base.join(&file.path))
                                .expect("tracked file path should render for users")
                        )
                    })?;
            }
            for directory in &node.directories {
                self.restore_directory(directory, &repo_relative_paths, &node.base)
                    .wrap_err_with(|| {
                        format!(
                            "failed to restore directory '{}'",
                            self.to_user_facing_path(node.base.join(&directory.path))
                                .expect("tracked directory path should render for users")
                        )
                    })?;
            }
        }

        Ok(())
    }

    fn restore_file(
        &self,
        file: &File,
        selected_paths: &[impl AsRef<Utf8Path> + Debug],
        base: &Utf8Path,
    ) -> Result<()> {
        let file_path = base.join(&file.path);
        if !selected_paths
            .iter()
            .any(|path| file_path.starts_with(path.as_ref()))
        {
            return Ok(());
        }

        let cached_path = self.cache_path_for(file.hash);
        if !cached_path.exists() {
            bail!(
                "{} is not in the cache",
                self.to_user_facing_path(&file_path)?
            );
        }

        let target = self.root.join(&file_path);
        debug!("restoring file '{target}'");
        if let Some(parent) = target.parent() {
            create_dir_all(parent)
                .wrap_err_with(|| format!("failed to create parent directories for '{target}'"))?;
        }
        copy(&cached_path, &target)
            .wrap_err_with(|| format!("failed to copy '{cached_path}' to '{target}'"))?;

        Ok(())
    }

    fn restore_directory(
        &self,
        directory: &Directory,
        selected_paths: &[impl AsRef<Utf8Path> + Debug],
        base: &Utf8Path,
    ) -> Result<()> {
        let directory_path = base.join(&directory.path);
        if !selected_paths.iter().any(|path| {
            let path = path.as_ref();
            path.starts_with(&directory_path) || directory_path.starts_with(path)
        }) {
            return Ok(());
        }

        if let DirectoryChildren::Resolved(children) = &directory.children {
            for child in children {
                self.restore_file(child, selected_paths, &directory_path)
                    .wrap_err_with(|| {
                        format!(
                            "failed to restore file '{}'",
                            self.to_user_facing_path(directory_path.join(&child.path))
                                .expect("tracked child path should render for users")
                        )
                    })?;
            }
        } else {
            warn!(
                "directory '{}' is not cached, cannot restore children",
                self.to_user_facing_path(&directory_path)
                    .expect("tracked directory path should render for users")
            );
        }
        Ok(())
    }
}

fn index_contains_path(index: &Index, selected_path: &Utf8Path) -> bool {
    index.iter_nodes().any(|node| {
        node.files
            .iter()
            .any(|file| node.base.join(&file.path).starts_with(selected_path))
            || node.directories.iter().any(|directory| {
                let directory_path = node.base.join(&directory.path);

                if directory_path.starts_with(selected_path) {
                    return true;
                }

                let children = match &directory.children {
                    DirectoryChildren::Resolved(children) => children,
                    DirectoryChildren::NotInCache => {
                        return selected_path.starts_with(&directory_path);
                    }
                };

                children
                    .iter()
                    .any(|child| directory_path.join(&child.path).starts_with(selected_path))
            })
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{reference_file::ReferenceFile, test_utils::setup_repo};

    use super::*;

    #[test]
    fn restore_single_file() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let path = "file.txt";
        fs::write(path, "file content")?;

        repo.add(["file.txt"])?;
        assert_eq!(fs::read_to_string(path)?, "file content");

        fs::write(path, "modified content")?;
        assert_eq!(fs::read_to_string(path)?, "modified content");

        repo.restore(&["file.txt"])?;
        assert_eq!(fs::read_to_string(path)?, "file content");

        Ok(())
    }

    #[test]
    fn restore_nested_file() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let path = "foo/bar/data.txt";
        fs::create_dir_all("foo/bar")?;
        fs::write(path, "nested file content")?;

        repo.add(["foo/bar/data.txt"])?;
        assert_eq!(fs::read_to_string(path)?, "nested file content");

        fs::write(path, "modified content")?;
        assert_eq!(fs::read_to_string(path)?, "modified content");

        repo.restore(&["foo"])?;
        assert_eq!(fs::read_to_string(path)?, "nested file content");

        Ok(())
    }

    #[test]
    fn restore_bare_and_nested_under_cwd_files_from_nested_cwd() -> Result<()> {
        let (repo, _tmp, mut guard) = setup_repo()?;

        fs::create_dir_all("nested/deeper")?;
        fs::write("nested/deeper/file.txt", "file content")?;
        fs::create_dir_all("nested/deeper/child")?;
        fs::write("nested/deeper/child/file.txt", "child file content")?;
        repo.add(["nested/deeper/file.txt", "nested/deeper/child/file.txt"])?;

        fs::write("nested/deeper/file.txt", "modified content")?;
        fs::write("nested/deeper/child/file.txt", "modified child content")?;

        let nested_dir = repo.root.join("nested/deeper");
        guard.change_to(&nested_dir)?;
        repo.restore(&[Utf8Path::new("file.txt"), Utf8Path::new("child/file.txt")])?;

        assert_eq!(
            fs::read_to_string(repo.root.join("nested/deeper/file.txt"))?,
            "file content"
        );
        assert!(
            !repo
                .root
                .join("nested/deeper/nested/deeper/file.txt")
                .exists(),
            "restore should not create duplicated nested paths"
        );
        assert_eq!(
            fs::read_to_string(repo.root.join("nested/deeper/child/file.txt"))?,
            "child file content"
        );

        Ok(())
    }

    #[test]
    fn restore_repo_relative_looking_file_from_nested_cwd_errors_when_selection_is_missing(
    ) -> Result<()> {
        let (repo, _tmp, mut guard) = setup_repo()?;

        fs::create_dir_all("nested/deeper")?;
        fs::write("nested/deeper/file.txt", "file content")?;
        repo.add(["nested/deeper/file.txt"])?;
        fs::write("nested/deeper/file.txt", "modified content")?;

        let nested_dir = repo.root.join("nested/deeper");
        guard.change_to(&nested_dir)?;

        let result = repo.restore(&[Utf8Path::new("nested/deeper/file.txt")]);

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("path 'nested/deeper/file.txt' is not tracked"));
        assert_eq!(
            fs::read_to_string(repo.root.join("nested/deeper/file.txt"))?,
            "modified content"
        );

        Ok(())
    }

    #[test]
    fn restore_directory_under_cwd_from_nested_cwd() -> Result<()> {
        let (repo, _tmp, mut guard) = setup_repo()?;

        let tracked_dir = repo.root.join("nested/deeper/tracked");
        let file_a = tracked_dir.join("a.txt");
        let file_b = tracked_dir.join("b.txt");

        fs::create_dir_all(&tracked_dir)?;
        fs::write(&file_a, "content a")?;
        fs::write(&file_b, "content b")?;
        repo.add([Utf8Path::new("nested/deeper/tracked")])?;

        fs::write(&file_a, "modified a")?;
        fs::write(&file_b, "modified b")?;

        let nested_dir = repo.root.join("nested/deeper");
        fs::create_dir_all(&nested_dir)?;
        guard.change_to(&nested_dir)?;

        repo.restore(&[Utf8Path::new("tracked")])?;

        assert_eq!(fs::read_to_string(&file_a)?, "content a");
        assert_eq!(fs::read_to_string(&file_b)?, "content b");
        assert!(
            !nested_dir.join("nested/deeper/tracked/a.txt").exists(),
            "restore should stay anchored at the repository root"
        );

        Ok(())
    }

    #[test]
    fn restore_multiple_files() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::write("a.txt", "content a")?;
        fs::write("b.txt", "content b")?;

        repo.add(["a.txt", "b.txt"])?;
        assert_eq!(fs::read_to_string("a.txt")?, "content a");
        assert_eq!(fs::read_to_string("b.txt")?, "content b");

        fs::write("a.txt", "modified a")?;
        fs::write("b.txt", "modified b")?;
        assert_eq!(fs::read_to_string("a.txt")?, "modified a");
        assert_eq!(fs::read_to_string("b.txt")?, "modified b");

        repo.restore(&["a.txt", "b.txt"])?;
        assert_eq!(fs::read_to_string("a.txt")?, "content a");
        assert_eq!(fs::read_to_string("b.txt")?, "content b");

        Ok(())
    }

    #[test]
    fn restore_directory() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::create_dir_all("dir")?;
        fs::write("dir/a.txt", "content a")?;
        fs::write("dir/b.txt", "content b")?;

        repo.add(["dir"])?;
        assert_eq!(fs::read_to_string("dir/a.txt")?, "content a");
        assert_eq!(fs::read_to_string("dir/b.txt")?, "content b");

        fs::write("dir/a.txt", "modified a")?;
        fs::write("dir/b.txt", "modified b")?;
        assert_eq!(fs::read_to_string("dir/a.txt")?, "modified a");
        assert_eq!(fs::read_to_string("dir/b.txt")?, "modified b");

        repo.restore(&["dir"])?;
        assert_eq!(fs::read_to_string("dir/a.txt")?, "content a");
        assert_eq!(fs::read_to_string("dir/b.txt")?, "content b");

        Ok(())
    }

    #[test]
    fn restore_nonexistent_file() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        // Attempt to restore a file that was never added
        let result = repo.restore(&["nonexistent.txt"]);
        assert!(result.is_err(), "restoring nonexistent file should error");

        Ok(())
    }

    #[test]
    fn restore_partial_directory() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::create_dir_all("data")?;
        fs::write("data/file1.txt", "content 1")?;
        fs::write("data/file2.txt", "content 2")?;

        repo.add(["data"])?;
        assert_eq!(fs::read_to_string("data/file1.txt")?, "content 1");
        assert_eq!(fs::read_to_string("data/file2.txt")?, "content 2");

        fs::write("data/file1.txt", "modified 1")?;
        fs::write("data/file2.txt", "modified 2")?;

        repo.restore(&["data/file2.txt"])?;
        assert_eq!(fs::read_to_string("data/file1.txt")?, "modified 1");
        assert_eq!(fs::read_to_string("data/file2.txt")?, "content 2");

        Ok(())
    }

    #[test]
    fn restore_missing_child_under_tracked_directory_errors() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::create_dir_all("tracked")?;
        fs::write("tracked/present.txt", "content")?;
        repo.add(["tracked"])?;

        let result = repo.restore(&["tracked/missing.txt"]);

        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn restore_tracked_child_under_missing_cache_directory_reaches_cache_miss_path() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        ReferenceFile::empty()
            .add_directory(crate::Reference::new(
                "tracked".into(),
                crate::hashing::Hash::from_bytes([7; 32]),
            ))
            .write(repo.root.join("tracked.jiji"))?;

        let result = repo.restore(&["tracked/present.txt"]);

        assert!(result.is_ok());
        assert!(!repo.root.join("tracked/present.txt").exists());

        Ok(())
    }

    #[test]
    fn restore_missing_cache_file_reports_path_relative_to_current_working_directory() -> Result<()>
    {
        let (repo, _tmp, mut guard) = setup_repo()?;

        fs::create_dir_all("nested/deeper")?;
        fs::write("nested/deeper/file.txt", "file content")?;
        let index = repo.add(["nested/deeper/file.txt"])?;
        let file = &index.iter_nodes().next().expect("node exists").files[0];
        fs::remove_file(repo.cache_path_for(file.hash))?;

        let nested_dir = repo.root.join("nested/deeper");
        guard.change_to(&nested_dir)?;

        let result = repo.restore(&[Utf8Path::new("file.txt")]);

        assert!(result.is_err());
        let error = format!("{:?}", result.unwrap_err());
        assert!(error.contains("file.txt is not in the cache"));
        assert!(!error.contains("nested/deeper/file.txt is not in the cache"));

        Ok(())
    }

    #[test]
    fn restore_ancestor_selection_restores_tracked_directory_beneath_it() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::create_dir_all("foo/bar")?;
        fs::write("foo/bar/data.txt", "original content")?;
        repo.add(["foo/bar"])?;

        fs::write("foo/bar/data.txt", "modified content")?;

        repo.restore(&["foo"])?;

        assert_eq!(fs::read_to_string("foo/bar/data.txt")?, "original content");

        Ok(())
    }

    #[test]
    fn restore_from_multiple_nodes() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::create_dir_all("dir")?;
        fs::write("dir/file1.txt", "file1 content")?;
        repo.add(["dir/file1.txt"])?;

        fs::write("dir/file2.txt", "file2 content")?;
        repo.add(["dir/file2.txt"])?;

        fs::write("dir/file1.txt", "modified file1")?;
        fs::write("dir/file2.txt", "modified file2")?;

        repo.restore(&["dir"])?;
        assert_eq!(fs::read_to_string("dir/file1.txt")?, "file1 content");
        assert_eq!(fs::read_to_string("dir/file2.txt")?, "file2 content");

        Ok(())
    }
}
