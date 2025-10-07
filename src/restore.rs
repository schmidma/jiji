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
    index::{Directory, DirectoryChildren, File},
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

        for node in index.iter_nodes() {
            for file in &node.files {
                self.restore_file(file, &repo_relative_paths, &node.base)
                    .wrap_err_with(|| {
                        format!("failed to restore file '{}'", node.base.join(&file.path))
                    })?;
            }
            for directory in &node.directories {
                self.restore_directory(directory, &repo_relative_paths, &node.base)
                    .wrap_err_with(|| {
                        format!("failed to restore directory '{}'", directory.path)
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
            bail!("{file_path} is not in the cache");
        }

        let target = &file_path;
        debug!("restoring file '{target}'");
        if let Some(parent) = target.parent() {
            create_dir_all(parent)
                .wrap_err_with(|| format!("failed to create parent directories for '{target}'"))?;
        }
        copy(&cached_path, target)
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
        if !selected_paths
            .iter()
            .any(|path| path.as_ref().starts_with(&directory_path))
        {
            return Ok(());
        }

        if let DirectoryChildren::Resolved(children) = &directory.children {
            for child in children {
                self.restore_file(child, selected_paths, &directory_path)
                    .wrap_err_with(|| format!("failed to restore file '{}'", child.path))?;
            }
        } else {
            warn!(
                "directory '{}' is not cached, cannot restore children",
                directory.path
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use std::fs;

    use crate::test_utils::setup_repo;

    use super::*;

    #[test]
    fn restore_single_file() -> Result<()> {
        let (repo, _tmp) = setup_repo()?;

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
        let (repo, _tmp) = setup_repo()?;

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
    fn restore_multiple_files() -> Result<()> {
        let (repo, _tmp) = setup_repo()?;

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
        let (repo, _tmp) = setup_repo()?;

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
        let (repo, _tmp) = setup_repo()?;

        // Attempt to restore a file that was never added
        let result = repo.restore(&["nonexistent.txt"]);
        assert!(
            result.is_ok(),
            "restoring nonexistent file should not error"
        );

        Ok(())
    }

    #[test]
    fn restore_partial_directory() -> Result<()> {
        let (repo, _tmp) = setup_repo()?;

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
    fn restore_from_multiple_nodes() -> Result<()> {
        let (repo, _tmp) = setup_repo()?;

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
