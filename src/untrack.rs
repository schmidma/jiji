use std::{fmt::Debug, fs};

use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    eyre::{bail, Context as _, ContextCompat as _},
    Result,
};

use crate::{
    index::{DirectoryChildren, Index, Node},
    reference_file::{Reference, ReferenceFile},
    JijiRepository,
};

impl JijiRepository {
    pub fn untrack(&self, paths: &[impl AsRef<Utf8Path> + Debug]) -> Result<()> {
        if paths.is_empty() {
            bail!("at least one path is required");
        }

        let _guard = self.write_lock("untrack")?;

        let selected_paths = paths
            .iter()
            .map(|path| self.to_repo_relative_path(path.as_ref()))
            .collect::<Result<Vec<_>>>()?;

        let mut index = self.index().wrap_err("failed to index repository")?;
        validate_selected_paths(&index, &selected_paths, self)?;

        for node in index.iter_nodes_mut() {
            if untrack_node(self, node, &selected_paths)? {
                persist_node_after_untrack(self, node)?;
            }
        }

        for path in selected_paths {
            println!("untracked {}", self.to_user_facing_path(path)?);
        }

        Ok(())
    }
}

fn validate_selected_paths(
    index: &Index,
    selected_paths: &[Utf8PathBuf],
    repo: &JijiRepository,
) -> Result<()> {
    for selected_path in selected_paths {
        if selected_path.as_str().is_empty() {
            bail!("path '' is not tracked");
        }

        if !index_contains_selected_path(index, selected_path)? {
            bail!(
                "path '{}' is not tracked",
                repo.to_user_facing_path(selected_path)?
            );
        }
    }

    Ok(())
}

fn index_contains_selected_path(index: &Index, selected_path: &Utf8Path) -> Result<bool> {
    for node in index.iter_nodes() {
        if node
            .files
            .iter()
            .any(|file| node.base.join(&file.path).starts_with(selected_path))
        {
            return Ok(true);
        }

        for directory in &node.directories {
            let directory_path = node.base.join(&directory.path);

            if directory_path.starts_with(selected_path) {
                return Ok(true);
            }

            if selected_path.starts_with(&directory_path) {
                let DirectoryChildren::Resolved(children) = &directory.children else {
                    bail!("directory manifest for '{directory_path}' is not in cache");
                };

                if children
                    .iter()
                    .any(|child| directory_path.join(&child.path).starts_with(selected_path))
                {
                    return Ok(true);
                }
            }
        }
    }

    Ok(false)
}

fn untrack_node(
    repo: &JijiRepository,
    node: &mut Node,
    selected_paths: &[Utf8PathBuf],
) -> Result<bool> {
    let original_file_count = node.files.len();
    let base = node.base.clone();
    node.files.retain(|file| {
        !selected_paths
            .iter()
            .any(|path| base.join(&file.path).starts_with(path))
    });

    let mut changed = node.files.len() != original_file_count;
    let mut directories = Vec::with_capacity(node.directories.len());
    for mut directory in node.directories.drain(..) {
        let directory_path = base.join(&directory.path);

        if selected_paths
            .iter()
            .any(|path| directory_path.starts_with(path))
        {
            changed = true;
            continue;
        }

        let mut children_changed = false;
        if selected_paths
            .iter()
            .any(|path| path.starts_with(&directory_path))
        {
            let DirectoryChildren::Resolved(children) = &mut directory.children else {
                bail!("directory manifest for '{directory_path}' is not in cache");
            };

            let original_child_count = children.len();
            children.retain(|child| {
                !selected_paths
                    .iter()
                    .any(|path| directory_path.join(&child.path).starts_with(path))
            });
            children_changed = children.len() != original_child_count;
        }

        if children_changed {
            let DirectoryChildren::Resolved(children) = &directory.children else {
                unreachable!("children were resolved before filtering");
            };
            let reference_file = ReferenceFile {
                files: children
                    .iter()
                    .map(|child| Reference::new(child.path.clone(), child.hash))
                    .collect(),
                directories: Vec::new(),
            };
            let new_hash = repo
                .cache_reference_file(&reference_file)
                .wrap_err_with(|| {
                    format!("failed to cache reference file for directory '{directory_path}'")
                })?;
            directory.hash = Some(new_hash);
            changed = true;
        }

        directories.push(directory);
    }
    node.directories = directories;

    Ok(changed)
}

fn persist_node_after_untrack(repo: &JijiRepository, node: &Node) -> Result<()> {
    let full_path = repo.root.join(&node.path);

    if node.files.is_empty() && node.directories.is_empty() {
        if full_path.exists() {
            fs::remove_file(&full_path)
                .wrap_err_with(|| format!("failed to remove reference file {full_path}"))?;
        }
        return Ok(());
    }

    let reference_file = ReferenceFile {
        files: node
            .files
            .iter()
            .map(|file| Reference::new(file.path.clone(), file.hash))
            .collect(),
        directories: node
            .directories
            .iter()
            .map(|directory| {
                let hash = directory.hash.with_context(|| {
                    format!("directory '{}' has no cached manifest hash", directory.path)
                })?;
                Ok(Reference::new(directory.path.clone(), hash))
            })
            .collect::<Result<_>>()?,
    };

    reference_file
        .write(&full_path)
        .wrap_err_with(|| format!("failed to write reference file to {full_path}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::mpsc, thread, time::Duration};

    use color_eyre::Result;

    use crate::{locking::LockMode, reference_file::ReferenceFile, test_utils::setup_repo};

    #[test]
    fn untrack_direct_file_removes_reference_and_keeps_working_file() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::write("file.txt", "file content")?;
        repo.add(["file.txt"])?;

        repo.untrack(&["file.txt"])?;

        assert_eq!(fs::read_to_string("file.txt")?, "file content");
        assert!(!repo.root.join("file.txt.jiji").exists());
        assert!(repo.index()?.iter_nodes().next().is_none());

        Ok(())
    }

    #[test]
    fn untrack_direct_file_keeps_cached_object() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::write("file.txt", "file content")?;
        repo.add(["file.txt"])?;
        let reference_file = ReferenceFile::read(repo.root.join("file.txt.jiji"))?;
        let cache_path = repo.cache_path_for(reference_file.files[0].hash);

        repo.untrack(&["file.txt"])?;

        assert!(cache_path.exists());
        assert_eq!(fs::read_to_string(cache_path)?, "file content");

        Ok(())
    }

    #[test]
    fn untrack_blocks_while_read_lock_held() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::write("file.txt", "file content")?;
        repo.add(["file.txt"])?;

        let read_guard = repo.repository_lock()?.acquire(LockMode::Read, || {})?;
        let (finished_tx, finished_rx) = mpsc::channel();

        thread::scope(|scope| {
            scope.spawn(|| {
                finished_tx
                    .send(repo.untrack(&["file.txt"]))
                    .expect("untrack result should send");
            });

            thread::sleep(Duration::from_millis(150));
            assert!(
                finished_rx.try_recv().is_err(),
                "untrack should stay blocked while a read lock is held"
            );

            drop(read_guard);

            finished_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("untrack should complete after the read lock is released")
                .expect("untrack should succeed once the lock is available");
        });

        Ok(())
    }

    #[test]
    fn untrack_nested_file_from_nested_cwd_uses_cwd_relative_paths() -> Result<()> {
        let (repo, _tmp, mut guard) = setup_repo()?;
        let nested_dir = repo.root.join("nested/deeper");
        fs::create_dir_all(nested_dir.as_std_path())?;
        fs::write(nested_dir.join("file.txt"), "nested content")?;
        repo.add([repo.root.join("nested/deeper/file.txt")])?;

        guard.change_to(&nested_dir)?;
        repo.untrack(&["file.txt"])?;

        assert_eq!(
            fs::read_to_string(nested_dir.join("file.txt"))?,
            "nested content"
        );
        assert!(!repo.root.join("nested/deeper/file.txt.jiji").exists());

        Ok(())
    }

    #[test]
    fn untrack_missing_path_errors_before_mutating() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::write("file.txt", "file content")?;
        repo.add(["file.txt"])?;

        let error = repo.untrack(&["missing.txt"]).unwrap_err().to_string();

        assert!(error.contains("path 'missing.txt' is not tracked"));
        assert!(repo.root.join("file.txt.jiji").exists());
        let reference_file = ReferenceFile::read(repo.root.join("file.txt.jiji"))?;
        assert_eq!(reference_file.files.len(), 1);

        Ok(())
    }

    #[test]
    fn untrack_empty_path_errors_before_mutating() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::write("file.txt", "file content")?;
        repo.add(["file.txt"])?;

        let error = repo.untrack(&[""]).unwrap_err().to_string();

        assert!(error.contains("path '' is not tracked"));
        assert!(repo.root.join("file.txt.jiji").exists());
        let reference_file = ReferenceFile::read(repo.root.join("file.txt.jiji"))?;
        assert_eq!(reference_file.files.len(), 1);

        Ok(())
    }

    #[test]
    fn untrack_empty_path_list_errors() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let paths: [&str; 0] = [];
        let error = repo.untrack(&paths).unwrap_err().to_string();

        assert!(error.contains("at least one path is required"));

        Ok(())
    }

    #[test]
    fn untrack_tracked_directory_removes_reference_and_keeps_contents() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::create_dir_all("data/subdir")?;
        fs::write("data/file.txt", "file content")?;
        fs::write("data/subdir/nested.txt", "nested content")?;
        repo.add(["data"])?;
        let old_manifest = ReferenceFile::read(repo.root.join("data.jiji"))?.directories[0].hash;

        repo.untrack(&["data"])?;

        assert_eq!(fs::read_to_string("data/file.txt")?, "file content");
        assert_eq!(
            fs::read_to_string("data/subdir/nested.txt")?,
            "nested content"
        );
        assert!(!repo.root.join("data.jiji").exists());
        assert!(repo.cache_path_for(old_manifest).exists());

        Ok(())
    }

    #[test]
    fn untrack_child_inside_tracked_directory_rewrites_manifest() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::create_dir_all("data")?;
        fs::write("data/remove.txt", "remove")?;
        fs::write("data/keep.txt", "keep")?;
        repo.add(["data"])?;
        let old_manifest = ReferenceFile::read(repo.root.join("data.jiji"))?.directories[0].hash;

        repo.untrack(&["data/remove.txt"])?;

        assert_eq!(fs::read_to_string("data/remove.txt")?, "remove");
        assert_eq!(fs::read_to_string("data/keep.txt")?, "keep");
        assert!(repo.cache_path_for(old_manifest).exists());

        let reference_file = ReferenceFile::read(repo.root.join("data.jiji"))?;
        assert_eq!(reference_file.directories.len(), 1);
        let new_manifest = reference_file.directories[0].hash;
        assert_ne!(new_manifest, old_manifest);

        let cached_reference_file = ReferenceFile::read(repo.cache_path_for(new_manifest))?;
        assert_eq!(cached_reference_file.files.len(), 1);
        assert_eq!(cached_reference_file.files[0].path, "keep.txt");

        Ok(())
    }

    #[test]
    fn untrack_last_child_keeps_empty_tracked_directory_manifest() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::create_dir_all("data")?;
        fs::write("data/remove.txt", "remove")?;
        repo.add(["data"])?;

        repo.untrack(&["data/remove.txt"])?;

        assert!(repo.root.join("data.jiji").exists());
        let reference_file = ReferenceFile::read(repo.root.join("data.jiji"))?;
        assert_eq!(reference_file.directories.len(), 1);

        let cached_reference_file =
            ReferenceFile::read(repo.cache_path_for(reference_file.directories[0].hash))?;
        assert!(cached_reference_file.files.is_empty());

        Ok(())
    }

    #[test]
    fn untrack_child_in_uncached_tracked_directory_errors_before_mutating() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::create_dir_all("data")?;
        fs::write("data/file.txt", "file content")?;
        repo.add(["data"])?;
        let reference_file = ReferenceFile::read(repo.root.join("data.jiji"))?;
        let manifest = reference_file.directories[0].hash;
        fs::remove_file(repo.cache_path_for(manifest))?;

        let error = repo.untrack(&["data/file.txt"]).unwrap_err().to_string();

        assert!(error.contains("directory manifest for 'data' is not in cache"));
        assert!(repo.root.join("data.jiji").exists());

        Ok(())
    }
}
