use std::fs;

use camino::Utf8Path;
use color_eyre::{
    eyre::{bail, Context as _},
    Result,
};

use crate::{
    hashing::hash_file,
    index::{Index, Node},
    with_added_extension::WithAddedExtension as _,
    JijiRepository,
};

impl JijiRepository {
    pub fn add(&self, paths: impl IntoIterator<Item = impl AsRef<Utf8Path>>) -> Result<Index> {
        let mut index = self.index().wrap_err("failed to index repository")?;
        self.add_with_index(&mut index, paths)?;
        Ok(index)
    }

    pub fn add_with_index(
        &self,
        index: &mut Index,
        paths: impl IntoIterator<Item = impl AsRef<Utf8Path>>,
    ) -> Result<()> {
        for path in paths {
            let path = path.as_ref();
            let relative_path = self.to_repo_relative_path(path)?;
            let user_facing_path = self.to_user_facing_path(&relative_path)?;
            self.add_path(index, &relative_path)
                .wrap_err_with(|| format!("failed to add {user_facing_path}"))?;
            println!("adding {user_facing_path}");
        }
        Ok(())
    }

    fn add_path(&self, index: &mut Index, path: &Utf8Path) -> Result<()> {
        let repository_path = self.root.join(path);
        let node = match index.find_owner_mut(path) {
            Some(node) => node,
            None => {
                let base = path.parent().unwrap_or_else(|| Utf8Path::new(""));
                index.push(Node {
                    path: path.with_added_extension("jiji"),
                    base: base.into(),
                    files: Vec::new(),
                    directories: Vec::new(),
                })
            }
        };

        let metadata = fs::metadata(&repository_path).wrap_err("failed to get metadata")?;
        if metadata.is_file() {
            let hash = hash_file(&repository_path)
                .wrap_err_with(|| format!("failed to hash file {path}"))?;
            let relative_path = path
                .strip_prefix(&node.base)
                .expect("path must be relative to base");
            node.add_file(relative_path, hash)
                .wrap_err_with(|| format!("failed to add file {path}"))?;
        } else if metadata.is_dir() {
            let directory = node
                .add_directory(path)
                .wrap_err_with(|| format!("failed to add directory {path}"))?;
            for entry in walkdir::WalkDir::new(self.root.join(path))
                .min_depth(1)
                .sort_by_file_name()
            {
                let entry = entry.wrap_err("failed to read directory entry")?;
                let entry_path: &Utf8Path = entry
                    .path()
                    .try_into()
                    .wrap_err("path is not valid utf-8")?;
                let child_path = entry_path
                    .strip_prefix(self.root.join(path))
                    .expect("entry is child of path");

                if entry.file_type().is_file() {
                    let hash = hash_file(entry_path)
                        .wrap_err_with(|| format!("failed to hash file {entry_path}"))?;
                    directory.add_file(child_path, hash).wrap_err_with(|| {
                        format!("failed to add file {child_path} to directory {path}")
                    })?;
                } else if entry.file_type().is_dir() {
                    // Directories will be handled implicitly by walking deeper
                } else {
                    bail!(
                        "unsupported file type for {entry_path}: {:?}",
                        entry.file_type()
                    );
                }
            }
        } else {
            bail!("expected file or directory, found {path} with metadata {metadata:?}");
        }

        node.persist_to_disk(self).wrap_err_with(|| {
            format!(
                "failed to persist reference file for node with path {}",
                node.path
            )
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{reference_file::ReferenceFile, test_utils::setup_repo};

    use super::*;

    #[test]
    fn add_single_file() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let path = "file.txt";
        fs::write(path, "file content")?;

        repo.add([path])?;

        assert!(repo.root.join(path).exists(), "file should still exist");
        assert!(
            repo.root.join("file.txt.jiji").exists(),
            "reference file should be created"
        );
        let reference_file = ReferenceFile::read(repo.root.join("file.txt.jiji"))?;
        assert_eq!(
            reference_file.files.len(),
            1,
            "reference file should contain one file"
        );
        assert_eq!(
            reference_file.files[0].path, path,
            "reference file path should match"
        );
        assert!(
            reference_file.directories.is_empty(),
            "reference file should not contain directories"
        );

        Ok(())
    }

    #[test]
    fn add_single_file_absolute() -> Result<()> {
        let (repo, tmp, _guard) = setup_repo()?;

        let tmp_path = <&Utf8Path>::try_from(tmp.path())?;
        let path = tmp_path.join("file.txt");
        fs::write(&path, "file content")?;

        repo.add([&path])?;

        assert!(path.exists(), "file should still exist");
        let ref_path = path.with_added_extension("jiji");
        assert!(ref_path.exists(), "reference file should be created");
        let reference_file = ReferenceFile::read(ref_path)?;
        assert_eq!(
            reference_file.files.len(),
            1,
            "reference file should contain one file"
        );
        assert_eq!(
            reference_file.files[0].path, "file.txt",
            "reference file path should match"
        );
        assert!(
            reference_file.directories.is_empty(),
            "reference file should not contain directories"
        );

        Ok(())
    }

    #[test]
    fn add_nested_file() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let path = "dir/subdir/file.txt";
        fs::create_dir_all("dir/subdir")?;
        fs::write(path, "file content")?;

        repo.add([path])?;

        assert!(repo.root.join(path).exists(), "file should still exist");
        let ref_path = path.with_added_extension("jiji");
        assert!(ref_path.exists(), "reference file should be created");
        let reference_file = ReferenceFile::read(ref_path)?;
        assert_eq!(
            reference_file.files.len(),
            1,
            "reference file should contain one file"
        );
        assert_eq!(
            reference_file.files[0].path, "file.txt",
            "reference file path should match"
        );
        assert!(
            reference_file.directories.is_empty(),
            "reference file should not contain directories"
        );

        Ok(())
    }

    #[test]
    fn add_bare_and_nested_under_cwd_paths_from_nested_cwd() -> Result<()> {
        let (repo, _tmp, mut guard) = setup_repo()?;

        let nested_dir = repo.root.join("nested/deeper");
        let nested_child_dir = nested_dir.join("child");
        fs::create_dir_all(nested_child_dir.as_std_path())?;
        fs::write(nested_dir.join("file.txt"), "file content")?;
        fs::write(nested_child_dir.join("file.txt"), "child file content")?;

        guard.change_to(&nested_dir)?;
        repo.add([Utf8Path::new("file.txt"), Utf8Path::new("child/file.txt")])?;

        let reference_file = ReferenceFile::read(repo.root.join("nested/deeper/file.txt.jiji"))?;
        assert_eq!(reference_file.files.len(), 1);
        assert_eq!(reference_file.files[0].path, "file.txt");

        let child_reference_file =
            ReferenceFile::read(repo.root.join("nested/deeper/child/file.txt.jiji"))?;
        assert_eq!(child_reference_file.files.len(), 1);
        assert_eq!(child_reference_file.files[0].path, "file.txt");

        Ok(())
    }

    #[test]
    fn add_repo_relative_looking_path_from_nested_cwd_errors_when_cwd_relative_target_is_missing(
    ) -> Result<()> {
        let (repo, _tmp, mut guard) = setup_repo()?;

        let nested_dir = repo.root.join("nested/deeper");
        fs::create_dir_all(nested_dir.as_std_path())?;
        fs::write(nested_dir.join("file.txt"), "file content")?;

        guard.change_to(&nested_dir)?;
        let result = repo.add([Utf8Path::new("nested/deeper/file.txt")]);

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("failed to add nested/deeper/file.txt"));
        assert!(!repo
            .root
            .join("nested/deeper/nested/deeper/file.txt.jiji")
            .exists());

        Ok(())
    }

    #[test]
    fn add_multiple_files() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let paths = ["file1.txt", "file2.txt", "dir/file3.txt"];
        for path in &paths {
            let path = Utf8Path::new(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, format!("content of {path}"))?;
        }

        repo.add(paths)?;

        for path in paths {
            assert!(repo.root.join(path).exists(), "file {path} should exist");
            let ref_path = path.with_added_extension("jiji");
            assert!(
                repo.root.join(&ref_path).exists(),
                "reference file {ref_path} should be created"
            );
            let reference_file = ReferenceFile::read(repo.root.join(&ref_path))?;
            assert_eq!(
                reference_file.files.len(),
                1,
                "reference file {ref_path} should contain one file"
            );
            let path = Utf8Path::new(path);
            assert_eq!(
                reference_file.files[0].path,
                path.file_name().unwrap(),
                "reference file path should match for {ref_path}"
            );
            assert!(
                reference_file.directories.is_empty(),
                "reference file {ref_path} should not contain directories"
            );
        }

        Ok(())
    }

    #[test]
    fn add_directory() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let dir_path = "mydir";
        let file_paths = ["mydir/file1.txt", "mydir/subdir/file2.txt"];
        for path in &file_paths {
            let path = Utf8Path::new(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, format!("content of {path}"))?;
        }

        repo.add([dir_path])?;

        assert!(repo.root.join(dir_path).exists(), "directory should exist");
        assert!(
            dir_path.with_added_extension("jiji").exists(),
            "reference file should be created"
        );
        let reference_file = ReferenceFile::read(dir_path.with_added_extension("jiji"))?;
        assert!(
            reference_file.files.is_empty(),
            "reference file should not contain files"
        );
        assert_eq!(
            reference_file.directories.len(),
            1,
            "reference file should contain one file"
        );
        assert_eq!(
            reference_file.directories[0].path, dir_path,
            "reference file path should match"
        );
        let hash = reference_file.directories[0].hash;
        let cached_path = repo.cache_path_for(hash);
        assert!(cached_path.exists(), "cached reference file should exist");
        let cached_reference_file = ReferenceFile::read(&cached_path)?;
        assert_eq!(
            cached_reference_file.files.len(),
            2,
            "cached reference file should contain two files"
        );
        assert_eq!(
            cached_reference_file.files[0].path, "file1.txt",
            "first file path should match"
        );
        assert_eq!(
            cached_reference_file.files[1].path, "subdir/file2.txt",
            "second file path should match"
        );
        assert!(
            cached_reference_file.directories.is_empty(),
            "cached reference file should not contain directories"
        );

        for path in &file_paths {
            assert!(repo.root.join(path).exists(), "file {path} should exist");
        }

        Ok(())
    }

    #[test]
    fn add_empty_directory() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let dir_path = "emptydir";
        fs::create_dir_all(dir_path)?;

        repo.add([dir_path])?;

        assert!(repo.root.join(dir_path).exists(), "directory should exist");
        assert!(
            !dir_path.with_added_extension("jiji").exists(),
            "reference file should not be created for empty directories"
        );

        Ok(())
    }

    #[test]
    fn add_update_existing_file() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let path = "file.txt";
        fs::write(path, "initial content")?;

        repo.add([path])?;

        let initial_ref_path = path.with_added_extension("jiji");
        assert!(
            initial_ref_path.exists(),
            "reference file should be created"
        );
        let initial_reference_file = ReferenceFile::read(&initial_ref_path)?;
        let initial_hash = initial_reference_file.files[0].hash;

        // Update the file content
        fs::write(path, "updated content")?;

        repo.add([path])?;

        assert!(repo.root.join(path).exists(), "file should still exist");
        assert!(
            initial_ref_path.exists(),
            "reference file should still exist"
        );
        let updated_reference_file = ReferenceFile::read(&initial_ref_path)?;
        let updated_hash = updated_reference_file.files[0].hash;

        assert_ne!(
            initial_hash, updated_hash,
            "hash should be updated after file change"
        );

        Ok(())
    }

    #[test]
    fn add_file_to_existing_directory() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let dir_path = "mydir";
        let initial_file_path = "mydir/file1.txt";
        fs::create_dir_all(dir_path)?;
        fs::write(initial_file_path, "initial content")?;

        repo.add([dir_path])?;

        let dir_ref_path = dir_path.with_added_extension("jiji");
        assert!(
            dir_ref_path.exists(),
            "reference file for directory should be created"
        );
        let initial_reference_file = ReferenceFile::read(&dir_ref_path)?;
        let initial_hash = initial_reference_file.directories[0].hash;

        // Add a new file to the existing directory
        let new_file_path = "mydir/file2.txt";
        fs::write(new_file_path, "new file content")?;

        repo.add([new_file_path])?;

        assert!(
            repo.root.join(new_file_path).exists(),
            "new file should still exist"
        );
        assert!(
            dir_ref_path.exists(),
            "reference file for directory should still exist"
        );
        let updated_reference_file = ReferenceFile::read(&dir_ref_path)?;
        let updated_hash = updated_reference_file.directories[0].hash;

        assert_ne!(
            initial_hash, updated_hash,
            "directory hash should be updated after adding new file"
        );

        let cached_path = repo.cache_path_for(updated_hash);
        assert!(cached_path.exists(), "cached reference file should exist");
        let cached_reference_file = ReferenceFile::read(&cached_path)?;
        assert_eq!(
            cached_reference_file.files.len(),
            2,
            "cached reference file should contain two files"
        );
        assert_eq!(
            cached_reference_file.files[0].path, "file1.txt",
            "first file path should match"
        );
        assert_eq!(
            cached_reference_file.files[1].path, "file2.txt",
            "second file path should match"
        );

        Ok(())
    }
}
