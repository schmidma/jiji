use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    eyre::{bail, Context as _, ContextCompat as _},
    Result,
};
use tracing::{debug, warn};
use walkdir::DirEntry;

use crate::{
    hashing::{hash_file, Hash},
    reference_file::{Reference, ReferenceFile},
    JijiRepository,
};

#[derive(Debug, PartialEq, Eq)]
pub struct Index {
    nodes: Vec<Node>,
}

impl Index {
    pub fn iter_nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter()
    }

    pub fn iter_nodes_mut(&mut self) -> impl Iterator<Item = &mut Node> {
        self.nodes.iter_mut()
    }

    pub fn find_owner_mut(&mut self, path: impl AsRef<Utf8Path>) -> Option<&mut Node> {
        let path = path.as_ref();
        self.nodes.iter_mut().find(|node| {
            node.files
                .iter()
                .any(|file| path == node.base.join(&file.path))
                || node
                    .directories
                    .iter()
                    .any(|dir| path.starts_with(node.base.join(&dir.path)))
        })
    }

    pub fn push(&mut self, node: Node) -> &mut Node {
        self.nodes.push(node);
        self.nodes.last_mut().unwrap()
    }

    pub fn resolve_status(&mut self, repo: &JijiRepository) -> Result<()> {
        for node in &mut self.nodes {
            node.resolve_status(repo)
                .wrap_err_with(|| format!("failed to resolve status for node '{}'", node.path))?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// Path to the reference file relative to the repository root, e.g. `data.jiji`.
    pub path: Utf8PathBuf,
    /// Base directory for this node: all file/dir entries are relative to `base`.
    pub base: Utf8PathBuf,
    pub files: Vec<File>,
    pub directories: Vec<Directory>,
}

impl Node {
    pub fn empty(path: impl Into<Utf8PathBuf>, base: impl Into<Utf8PathBuf>) -> Self {
        Self {
            path: path.into(),
            base: base.into(),
            files: Vec::new(),
            directories: Vec::new(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.files
            .iter()
            .any(|file| file.status == FileStatus::Staged)
            || self.directories.iter().any(Directory::is_dirty)
    }

    pub fn add_file(&mut self, path: &Utf8Path, hash: Hash) -> Result<()> {
        // 1) Check if the file is already tracked directly
        if let Some(file) = self.files.iter_mut().find(|file| file.path == path) {
            if file.hash == hash {
                debug!("file {path} is already tracked and up-to-date");
                return Ok(());
            }

            debug!("file {path} is tracked but modified");
            file.hash = hash;
            file.status = FileStatus::Staged;
            return Ok(());
        }

        // 2) Check if the file belongs to one of the node directories
        if let Some(directory) = self
            .directories
            .iter_mut()
            .find(|directory| path.starts_with(&directory.path))
        {
            let rel_to_dir = path
                .strip_prefix(&directory.path)
                .expect("file under directory");
            directory.add_file(rel_to_dir, hash).wrap_err_with(|| {
                format!(
                    "failed to add file '{path}' to tracked directory '{}'",
                    directory.path
                )
            })?;
            return Ok(());
        }

        // 3) Not tracked in node at all -> add as direct file
        self.files.push(File {
            path: path.into(),
            hash,
            status: FileStatus::Staged,
        });

        Ok(())
    }

    pub fn add_directory(&mut self, path: &Utf8Path) -> Result<&mut Directory> {
        // `path` is relative to this node's `base`, matching how directory entries are stored.
        // Check if this base-relative directory is already tracked by the node.
        if let Some((i, _)) = self
            .directories
            .iter()
            // Fighting the borrow checker here, we cannot do this by mutable reference, as the
            // borrow checker keeps the borrow alive in both branches. (probably fixed with
            // Polonius)
            .enumerate()
            .find(|(_i, dir)| dir.path == path)
        {
            debug!("directory '{path}' is already tracked");
            return Ok(self.directories.get_mut(i).expect("just found"));
        }

        // Reject a base-relative directory if tracked files already live under it.
        if self.files.iter().any(|file| file.path.starts_with(path)) {
            bail!("cannot add directory '{path}' because it contains tracked files");
        }

        // Reject overlapping base-relative directories in either direction.
        if let Some(nested) = self
            .directories
            .iter()
            .find(|dir| dir.path.starts_with(path) || path.starts_with(&dir.path))
        {
            bail!("cannot add directory '{path}' because it conflicts with existing tracked directory '{}'", nested.path);
        }

        self.directories.push(Directory {
            path: path.to_owned(),
            hash: None,
            children: DirectoryChildren::Resolved(Vec::new()),
        });
        Ok(self.directories.last_mut().expect("just added"))
    }

    pub fn persist_to_disk(&mut self, repo: &JijiRepository) -> Result<()> {
        if !self.is_dirty() {
            debug!("self '{}' is not dirty; skipping persist", self.path);
            return Ok(());
        }

        let files = self
            .files
            .iter_mut()
            .map(|file| {
                if file.status == FileStatus::Staged {
                    repo.cache_file(file.hash, self.base.join(&file.path))
                        .wrap_err_with(|| format!("failed to cache file '{}'", file.path))?;
                    file.status = FileStatus::Unknown;
                }
                Ok(Reference::new(file.path.clone(), file.hash))
            })
            .collect::<Result<_>>()
            .wrap_err_with(|| {
                format!("failed to process files for reference file '{}'", self.path)
            })?;

        let directories = self
            .directories
            .iter_mut()
            .map(|directory| {
                let DirectoryChildren::Resolved(files) = &mut directory.children else {
                    bail!("directory '{}' is not in cache", directory.path);
                };
                for file in files
                    .iter_mut()
                    .filter(|file| file.status == FileStatus::Staged)
                {
                    let path = self.base.join(&directory.path).join(&file.path);
                    repo.cache_file(file.hash, path)
                        .wrap_err_with(|| format!("failed to cache file '{}'", file.path))?;
                    file.status = FileStatus::Unknown;
                }

                let reference_file = ReferenceFile {
                    files: files
                        .iter()
                        .map(|file| Reference::new(file.path.clone(), file.hash))
                        .collect(),
                    directories: Vec::new(),
                };

                let dir_hash = repo
                    .cache_reference_file(&reference_file)
                    .wrap_err_with(|| {
                        format!(
                            "failed to cache reference file for directory '{}'",
                            directory.path
                        )
                    })?;

                Ok(Reference::new(directory.path.clone(), dir_hash))
            })
            .collect::<Result<_>>()
            .wrap_err_with(|| {
                format!(
                    "failed to process directories for reference file '{}'",
                    self.path
                )
            })?;

        let reference_file = ReferenceFile { files, directories };

        let full_path = repo.root.join(&self.path);
        reference_file
            .write(&full_path)
            .wrap_err_with(|| format!("failed to write reference file to {full_path}"))?;

        Ok(())
    }

    pub fn resolve_status(&mut self, repo: &JijiRepository) -> Result<()> {
        for file in &mut self.files {
            let base = repo.root.join(&self.base);
            file.resolve_status(&base)
                .wrap_err_with(|| format!("failed to resolve status for file '{}'", file.path))?;
        }
        for directory in &mut self.directories {
            let DirectoryChildren::Resolved(children) = &mut directory.children else {
                continue;
            };
            let base = repo.root.join(&self.base).join(&directory.path);
            for file in children {
                file.resolve_status(&base).wrap_err_with(|| {
                    format!(
                        "failed to resolve status for file '{}' in directory '{}'",
                        file.path, directory.path
                    )
                })?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    /// File is newly added or has been modified, caching is pending
    Staged,
    /// File exists on disk but its contents differ from the tracked hash
    Modified { hash_on_disk: Hash },
    /// File is missing on disk
    Deleted,
    /// File exists on disk and matches the tracked hash
    Clean,
    /// File status has not been resolved yet
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    /// Path relative to its parent (node for files, directory for directories)
    pub path: Utf8PathBuf,
    /// Hash as tracked in the reference file
    pub hash: Hash,
    pub status: FileStatus,
}

impl File {
    fn resolve_status(&mut self, base: &Utf8Path) -> Result<()> {
        if self.status != FileStatus::Unknown {
            // Already resolved
            return Ok(());
        }

        let full_path = base.join(&self.path);
        if !full_path.exists() {
            self.status = FileStatus::Deleted;
            return Ok(());
        }

        let hash_on_disk = hash_file(&full_path)
            .wrap_err_with(|| format!("failed to hash file at {full_path}"))?;
        if hash_on_disk != self.hash {
            self.status = FileStatus::Modified { hash_on_disk };
            return Ok(());
        }

        self.status = FileStatus::Clean;
        Ok(())
    }
}

impl From<Reference> for File {
    fn from(value: Reference) -> Self {
        Self {
            path: value.path,
            hash: value.hash,
            status: FileStatus::Unknown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectoryChildren {
    NotInCache,
    Resolved(Vec<File>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directory {
    /// Path relative to its parent node.
    pub path: Utf8PathBuf,
    pub hash: Option<Hash>,
    pub children: DirectoryChildren,
}

impl Directory {
    pub fn is_dirty(&self) -> bool {
        matches!(
            &self.children,
            DirectoryChildren::Resolved(children) if children.iter().any(|child| child.status == FileStatus::Staged)
        )
    }

    pub fn add_file(&mut self, path: &Utf8Path, hash: Hash) -> Result<()> {
        // Ensure children are present (manifest cached)
        let DirectoryChildren::Resolved(children) = &mut self.children else {
            bail!(
                "directory manifest for '{}' is not cached; cannot add children",
                self.path
            );
        };

        if let Some(child) = children.iter_mut().find(|child| child.path == path) {
            if child.hash == hash {
                debug!("file {path} (in dir) is already up-to-date");
                return Ok(());
            }

            debug!("file {path} (in dir) changed");
            child.hash = hash;
            child.status = FileStatus::Staged;
            return Ok(());
        }

        // Not found in children -> add as new child
        debug!(
            "file {path} is not tracked inside directory {}; adding",
            self.path,
        );
        children.push(File {
            path: path.to_owned(),
            hash,
            status: FileStatus::Staged,
        });

        Ok(())
    }
}

impl JijiRepository {
    pub fn index_directory(&self, reference: Reference) -> Result<Directory> {
        let cache_path = self.cache_path_for(reference.hash);
        let children = if cache_path.exists() {
            let reference_file = ReferenceFile::read(&cache_path)
                .wrap_err_with(|| format!("failed to read reference file at {cache_path}"))?;
            let files = reference_file.files.into_iter().map(File::from).collect();

            if !reference_file.directories.is_empty() {
                bail!("found nested directories in cached reference file at {cache_path}, this is not supported.");
            }
            DirectoryChildren::Resolved(files)
        } else {
            DirectoryChildren::NotInCache
        };

        Ok(Directory {
            path: reference.path,
            hash: Some(reference.hash),
            children,
        })
    }

    fn index_node(&self, path: &Utf8Path) -> Result<Node> {
        let path = path.strip_prefix(&self.root).wrap_err_with(|| {
            format!(
                "reference file path '{path}' is not under repository root '{}'",
                self.root
            )
        })?;
        let reference_file =
            ReferenceFile::read(self.root.join(path)).wrap_err("failed to read reference file")?;
        let base = path.parent().wrap_err("reference file has no parent")?;

        let files = reference_file.files.into_iter().map(File::from).collect();
        let directories = reference_file
            .directories
            .into_iter()
            .map(|reference| {
                self.index_directory(reference)
                    .wrap_err_with(|| format!("failed to index directory in {path}"))
            })
            .collect::<Result<_>>()
            .wrap_err("failed to index directories")?;

        Ok(Node {
            path: path.to_owned(),
            base: base.to_owned(),
            files,
            directories,
        })
    }

    pub fn index(&self) -> Result<Index> {
        let nodes = walkdir::WalkDir::new(&self.root)
            .sort_by_file_name()
            .into_iter()
            .filter_map(|entry| match entry {
                Ok(entry) => Some(entry),
                Err(error) => {
                    warn!("failed to read entry: {error:#}");
                    None
                }
            })
            .filter(is_reference_file)
            .map(|entry| {
                let path = entry
                    .path()
                    .try_into()
                    .wrap_err("path is not valid UTF-8")?;
                self.index_node(path)
                    .wrap_err_with(|| format!("failed to index reference file at {path}"))
            })
            .collect::<Result<_>>()?;
        Ok(Index { nodes })
    }
}

fn is_reference_file(entry: &DirEntry) -> bool {
    entry.file_type().is_file() && entry.path().extension().is_some_and(|ext| ext == "jiji")
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::hashing::hash_file;
    use crate::test_utils::setup_repo;
    use color_eyre::Result;
    use std::fs;
    use std::fs::create_dir_all;

    #[test]
    fn index_empty_repository() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;
        let index = repo.index()?;
        assert!(index.nodes.is_empty(), "index should be empty");
        Ok(())
    }

    #[test]
    fn index_single_file() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        ReferenceFile::empty()
            .add_file(Reference::new("file.txt".into(), Hash::from_bytes([1; 32])))
            .write(repo.root.join("file.txt.jiji"))?;

        let index = repo.index()?;

        assert_eq!(
            index.nodes.len(),
            1,
            "index should contain exactly one node"
        );

        let node = &index.nodes[0];
        assert_eq!(
            node.path, "file.txt.jiji",
            "node path should match reference file"
        );
        assert_eq!(
            node.base, "",
            "base path should be empty for top-level file"
        );
        assert!(!node.is_dirty(), "node should not be marked dirty");

        assert_eq!(node.files.len(), 1, "node should contain exactly one file");
        let file = &node.files[0];
        assert_eq!(
            file.path, "file.txt",
            "tracked file path should be relative"
        );
        assert_eq!(
            file.hash,
            Hash::from_bytes([1u8; 32]),
            "tracked file hash should match reference"
        );
        assert_eq!(file.status, FileStatus::Unknown, "file should be clean");

        assert!(
            node.directories.is_empty(),
            "node should not contain directories"
        );

        Ok(())
    }

    #[test]
    fn index_nested_paths() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let nested_path = repo.root.join("foo/bar/file.txt.jiji");
        create_dir_all(nested_path.parent().unwrap())?;

        ReferenceFile::empty()
            .add_file(Reference::new("file.txt".into(), Hash::from_bytes([1; 32])))
            .write(nested_path)?;

        let index = repo.index()?;

        assert_eq!(
            index.nodes.len(),
            1,
            "index should contain exactly one node"
        );

        let node = &index.nodes[0];
        assert_eq!(
            node.path, "foo/bar/file.txt.jiji",
            "node path should be nested path"
        );
        assert_eq!(
            node.base, "foo/bar",
            "base should match parent directory of reference file"
        );
        assert!(!node.is_dirty(), "node should not be marked dirty");

        assert_eq!(node.files.len(), 1, "node should contain exactly one file");
        let file = &node.files[0];
        assert_eq!(
            file.path, "file.txt",
            "file path should be relative to base"
        );
        assert_eq!(
            file.hash,
            Hash::from_bytes([1u8; 32]),
            "tracked file hash should match reference"
        );
        assert_eq!(file.status, FileStatus::Unknown, "file should be clean");

        assert!(
            node.directories.is_empty(),
            "node should not contain directories"
        );

        Ok(())
    }

    #[test]
    fn index_directory_not_in_cache() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        ReferenceFile::empty()
            .add_directory(Reference::new("data".into(), Hash::from_bytes([2; 32])))
            .write(repo.root.join("data.jiji"))?;

        let index = repo.index()?;
        assert_eq!(
            index.nodes.len(),
            1,
            "index should contain exactly one node"
        );

        let node = &index.nodes[0];
        assert_eq!(
            node.path, "data.jiji",
            "node path should match reference file"
        );
        assert_eq!(node.files.len(), 0, "node should contain no direct files");
        assert_eq!(
            node.directories.len(),
            1,
            "node should contain one directory entry"
        );

        match &node.directories[0].children {
            DirectoryChildren::NotInCache => {}
            other => panic!("expected NotInCache, got {other:?}"),
        }

        Ok(())
    }

    #[test]
    fn index_directory_in_cache_resolved() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let child_hash = Hash::from_bytes([10; 32]);
        let mut directory_manifest = ReferenceFile::empty();
        directory_manifest.add_file(Reference::new("file.txt".into(), child_hash));
        let directory_hash = repo.cache_reference_file(&directory_manifest)?;

        ReferenceFile::empty()
            .add_directory(Reference::new("data".into(), directory_hash))
            .write(repo.root.join("data.jiji"))?;

        let index = repo.index()?;
        assert_eq!(
            index.nodes.len(),
            1,
            "index should contain exactly one node"
        );

        let node = &index.nodes[0];
        assert_eq!(
            node.directories.len(),
            1,
            "node should contain one directory entry"
        );

        let DirectoryChildren::Resolved(children) = &node.directories[0].children else {
            panic!("expected directory to be resolved from cache");
        };

        assert_eq!(
            children.len(),
            1,
            "resolved children should contain one file"
        );
        assert_eq!(
            children[0].path, "file.txt",
            "child path should match cached manifest"
        );
        assert_eq!(
            children[0].hash, child_hash,
            "child hash should match cached manifest"
        );
        assert_eq!(
            children[0].status,
            FileStatus::Unknown,
            "child should be clean"
        );

        Ok(())
    }

    #[test]
    fn index_nested_tracked_directory_round_trips_base_relative_paths() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::create_dir_all(repo.root.join("foo/bar/images").as_std_path())?;
        fs::write(repo.root.join("foo/bar/images/photo.jpg"), "image content")?;

        repo.add(["foo/bar/images"])?;

        let reopened = JijiRepository::new(repo.root.clone())?;
        let index = reopened.index()?;
        assert_eq!(
            index.nodes.len(),
            1,
            "index should contain exactly one node"
        );

        let node = &index.nodes[0];
        assert_eq!(node.path, "foo/bar/images.jiji", "node path should match");
        assert_eq!(
            node.base, "foo/bar",
            "node base should match parent directory"
        );
        assert!(node.files.is_empty(), "node should have no direct files");
        assert_eq!(
            node.directories.len(),
            1,
            "node should contain one tracked directory"
        );
        assert_eq!(
            node.directories[0].path, "images",
            "directory path should remain relative to the node base"
        );

        let DirectoryChildren::Resolved(children) = &node.directories[0].children else {
            panic!("expected directory to be resolved from cache");
        };
        assert_eq!(
            children.len(),
            1,
            "tracked directory should contain one file"
        );
        assert_eq!(
            children[0].path, "photo.jpg",
            "tracked-directory child manifest should remain relative to the tracked directory root"
        );
        let photo_hash = hash_file(repo.root.join("foo/bar/images/photo.jpg"))?;
        assert_eq!(children[0].hash, photo_hash, "child hash should match");
        assert_eq!(
            children[0].status,
            FileStatus::Unknown,
            "indexed child should start unresolved"
        );

        Ok(())
    }

    #[test]
    fn resolve_status_for_nested_tracked_directory_after_reload() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::create_dir_all(repo.root.join("nested/dir").as_std_path())?;
        fs::write(repo.root.join("nested/dir/file.txt"), "original content")?;

        repo.add(["nested/dir"])?;

        fs::write(repo.root.join("nested/dir/file.txt"), "modified content")?;

        let reopened = JijiRepository::new(repo.root.clone())?;
        let mut index = reopened.index()?;
        index.resolve_status(&reopened)?;

        assert_eq!(
            index.nodes.len(),
            1,
            "index should contain exactly one node"
        );

        let node = &index.nodes[0];
        assert_eq!(node.base, "nested", "node base should round-trip");
        assert_eq!(
            node.directories.len(),
            1,
            "node should contain one tracked directory"
        );
        assert_eq!(
            node.directories[0].path, "dir",
            "directory path should remain base-relative after reload"
        );

        let DirectoryChildren::Resolved(children) = &node.directories[0].children else {
            panic!("expected directory children to resolve from cache");
        };
        assert_eq!(
            children.len(),
            1,
            "tracked directory should contain one file"
        );
        assert_eq!(
            children[0].path, "file.txt",
            "child path should stay relative"
        );
        assert!(
            matches!(children[0].status, FileStatus::Modified { .. }),
            "status resolution should inspect the nested tracked file after reload"
        );

        Ok(())
    }

    #[test]
    fn index_multiple_nodes() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        ReferenceFile::empty()
            .add_file(Reference::new("f1.txt".into(), Hash::from_bytes([1; 32])))
            .write(repo.root.join("one.jiji"))?;

        let nested_dir = repo.root.join("x/y");
        create_dir_all(&nested_dir)?;
        ReferenceFile::empty()
            .add_file(Reference::new("g.txt".into(), Hash::from_bytes([2; 32])))
            .write(nested_dir.join("two.jiji"))?;

        let index = repo.index()?;
        assert_eq!(index.nodes.len(), 2, "index should contain two nodes");

        let paths: Vec<_> = index.nodes.iter().map(|n| n.path.as_str()).collect();
        assert!(
            paths.contains(&"one.jiji"),
            "index nodes should contain top-level one.jiji"
        );
        assert!(
            paths.contains(&"x/y/two.jiji"),
            "index nodes should contain nested x/y/two.jiji"
        );

        Ok(())
    }

    #[test]
    fn find_owner_mut_matches_repo_relative_lookup_against_base_relative_file() {
        let mut index = Index {
            nodes: vec![Node {
                path: "foo/bar/data.jiji".into(),
                base: "foo/bar".into(),
                files: vec![File {
                    path: "file.txt".into(),
                    hash: Hash::from_bytes([1; 32]),
                    status: FileStatus::Unknown,
                }],
                directories: Vec::new(),
            }],
        };

        let owner = index.find_owner_mut("foo/bar/file.txt");

        assert!(
            owner.is_some(),
            "repo-relative lookup should match file stored relative to node base"
        );
    }

    #[test]
    fn find_owner_mut_matches_repo_relative_lookup_against_base_relative_directory() {
        let mut index = Index {
            nodes: vec![Node {
                path: "foo/bar/data.jiji".into(),
                base: "foo/bar".into(),
                files: Vec::new(),
                directories: vec![Directory {
                    path: "images".into(),
                    hash: None,
                    children: DirectoryChildren::Resolved(Vec::new()),
                }],
            }],
        };

        let owner = index.find_owner_mut("foo/bar/images/photo.jpg");

        assert!(
            owner.is_some(),
            "repo-relative lookup should match directory stored relative to node base"
        );
    }

    #[test]
    fn index_ignores_non_jiji_files() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::write(repo.root.join("not_a_ref.txt"), "hello")?;
        ReferenceFile::empty()
            .add_file(Reference::new("f.txt".into(), Hash::from_bytes([7; 32])))
            .write(repo.root.join("valid.jiji"))?;

        let index = repo.index()?;
        assert_eq!(
            index.nodes.len(),
            1,
            "only the .jiji reference should be indexed"
        );

        Ok(())
    }

    #[test]
    fn index_cached_manifest_with_nested_directories_errors() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        // Cached manifest with nested directories
        let mut bad_manifest = ReferenceFile::empty();
        bad_manifest.add_directory(Reference::new("inner".into(), Hash::from_bytes([0xaa; 32])));
        let bad_hash = repo.cache_reference_file(&bad_manifest)?;

        ReferenceFile::empty()
            .add_directory(Reference::new("data".into(), bad_hash))
            .write(repo.root.join("bad.jiji"))?;

        let res = repo.index();
        assert!(
            res.is_err(),
            "indexing should error when cached manifest contains nested directories"
        );

        Ok(())
    }

    #[test]
    fn add_file_to_empty_node() -> Result<()> {
        let file_path = Utf8Path::new("data.jpg");
        let file_hash = Hash::from_bytes([1u8; 32]);

        let mut node = Node::empty("foo/data.jpg.jiji", "foo");
        node.add_file(file_path, file_hash)?;

        assert_eq!(node.files.len(), 1, "node should contain one file");
        assert_eq!(node.files[0].path, "data.jpg", "correct relative path");
        assert_eq!(node.files[0].hash, file_hash, "correct hash recorded");
        assert_eq!(node.files[0].status, FileStatus::Staged, "status pending");
        assert!(node.is_dirty(), "node should be marked dirty");
        assert!(
            node.directories.is_empty(),
            "no directories should be present"
        );

        Ok(())
    }

    #[test]
    fn add_file_to_non_empty_node() -> Result<()> {
        let existing_hash = Hash::from_bytes([1u8; 32]);
        let mut node = Node {
            path: "foo/existing.jiji".into(),
            base: "foo".into(),
            files: vec![File {
                path: "existing.txt".into(),
                hash: existing_hash,
                status: FileStatus::Unknown,
            }],
            directories: Vec::new(),
        };

        let file_path = Utf8Path::new("data.jpg");
        let new_hash = Hash::from_bytes([2u8; 32]);
        node.add_file(file_path, new_hash)?;

        assert_eq!(node.files.len(), 2, "node should contain two files");
        assert_eq!(
            node.files[0].path, "existing.txt",
            "existing file still tracked"
        );
        assert_eq!(
            node.files[0].hash, existing_hash,
            "existing file hash unchanged"
        );
        assert_eq!(
            node.files[0].status,
            FileStatus::Unknown,
            "existing file still clean"
        );
        assert_eq!(node.files[1].path, "data.jpg", "new file added");
        assert_eq!(node.files[1].hash, new_hash, "new file has correct hash");
        assert_eq!(
            node.files[1].status,
            FileStatus::Staged,
            "new file is pending"
        );
        assert!(node.is_dirty(), "node should be marked dirty");

        Ok(())
    }

    #[test]
    fn update_file_in_node() -> Result<()> {
        let file_path = Utf8Path::new("data.jpg");

        let old_hash = Hash::from_bytes([1u8; 32]);
        let mut node = Node {
            path: "foo/data.jpg.jiji".into(),
            base: "foo".into(),
            files: vec![File {
                path: "data.jpg".into(),
                hash: old_hash,
                status: FileStatus::Unknown,
            }],
            directories: Vec::new(),
        };

        let new_hash = Hash::from_bytes([2u8; 32]);
        node.add_file(file_path, new_hash)?;

        assert_eq!(node.files[0].hash, new_hash, "file hash should be updated");
        assert_eq!(
            node.files[0].status,
            FileStatus::Staged,
            "file status should be pending"
        );
        assert!(node.is_dirty(), "node should be dirty after update");

        let snapshot = node.clone();
        node.add_file(file_path, new_hash)?;
        assert_eq!(node, snapshot, "re-adding unchanged file should be no-op");

        Ok(())
    }

    #[test]
    fn add_same_file_to_node() -> Result<()> {
        let file_path = Utf8Path::new("data.jpg");
        let hash = Hash::from_bytes([1u8; 32]);

        let mut node = Node {
            path: "foo/data.jpg.jiji".into(),
            base: "foo".into(),
            files: vec![File {
                path: "data.jpg".into(),
                hash,
                status: FileStatus::Unknown,
            }],
            directories: Vec::new(),
        };
        let snapshot = node.clone();

        node.add_file(file_path, hash)?;

        assert_eq!(node.files.len(), 1, "node should still contain one file");
        assert_eq!(node.files[0].hash, hash, "hash should not change");
        assert_eq!(
            node.files[0].status,
            FileStatus::Unknown,
            "status should not change"
        );
        assert!(!node.is_dirty(), "node should remain clean");
        assert_eq!(node, snapshot, "node should be unchanged");

        Ok(())
    }

    #[test]
    fn add_file_under_tracked_directory() -> Result<()> {
        let file_path = Utf8Path::new("foo/data.jpg");
        let hash = Hash::from_bytes([1u8; 32]);

        let mut node = Node {
            path: "foo.jiji".into(),
            base: "".into(),
            files: Vec::new(),
            directories: vec![Directory {
                path: "foo".into(),
                hash: None,
                children: DirectoryChildren::Resolved(Vec::new()),
            }],
        };

        node.add_file(file_path, hash)?;

        assert_eq!(node.files.len(), 0, "no direct files should be tracked");
        assert_eq!(node.directories.len(), 1, "one directory tracked");
        let directory = &node.directories[0];
        assert_eq!(directory.path, "foo", "directory path unchanged");
        let DirectoryChildren::Resolved(children) = &directory.children else {
            panic!("directory children should be resolved");
        };
        assert_eq!(children.len(), 1, "directory should contain one file");
        assert_eq!(children[0].path, "data.jpg", "file correctly added");
        assert_eq!(children[0].hash, hash, "correct file hash");
        assert_eq!(children[0].status, FileStatus::Staged, "status pending");
        assert!(directory.is_dirty(), "directory should be marked dirty");
        assert!(node.is_dirty(), "node should be marked dirty");

        Ok(())
    }

    #[test]
    fn update_file_inside_tracked_directory() -> Result<()> {
        let file_path = Utf8Path::new("foo/data.txt");
        let old_hash = Hash::from_bytes([1u8; 32]);

        let mut node = Node {
            path: "foo.jiji".into(),
            base: "".into(),
            files: Vec::new(),
            directories: vec![Directory {
                path: "foo".into(),
                hash: None,
                children: DirectoryChildren::Resolved(vec![File {
                    path: "data.txt".into(),
                    hash: old_hash,
                    status: FileStatus::Unknown,
                }]),
            }],
        };

        let new_hash = Hash::from_bytes([2u8; 32]);
        node.add_file(file_path, new_hash)?;

        assert_eq!(node.files.len(), 0, "no direct files should be tracked");
        assert_eq!(node.directories.len(), 1, "one directory tracked");
        let directory = &node.directories[0];
        let DirectoryChildren::Resolved(children) = &directory.children else {
            panic!("directory children should be resolved");
        };
        assert_eq!(children[0].hash, new_hash, "file hash updated");
        assert_eq!(children[0].status, FileStatus::Staged, "status pending");
        assert!(node.is_dirty(), "node marked dirty");
        assert!(directory.is_dirty(), "directory marked dirty");

        Ok(())
    }

    #[test]
    fn readd_unchanged_file_in_directory_is_noop() -> Result<()> {
        let file_path = Utf8Path::new("foo/data.txt");
        let hash = Hash::from_bytes([1u8; 32]);

        let mut node = Node {
            path: "foo.jiji".into(),
            base: "".into(),
            files: Vec::new(),
            directories: vec![Directory {
                path: "foo".into(),
                hash: None,
                children: DirectoryChildren::Resolved(vec![File {
                    path: "data.txt".into(),
                    hash,
                    status: FileStatus::Unknown,
                }]),
            }],
        };
        let snapshot = node.clone();

        node.add_file(file_path, hash)?;

        assert_eq!(node.files.len(), 0, "no direct files should be tracked");
        assert_eq!(node.directories.len(), 1, "one directory tracked");
        let directory = &node.directories[0];
        let DirectoryChildren::Resolved(children) = &directory.children else {
            panic!("directory children should be resolved");
        };
        assert_eq!(children[0].hash, hash, "file hash unchanged");
        assert_eq!(children[0].status, FileStatus::Unknown, "status unchanged");
        assert!(!directory.is_dirty(), "directory should not be dirty");
        assert!(!node.is_dirty(), "node should not be dirty");
        assert_eq!(node, snapshot, "node should be unchanged");

        Ok(())
    }

    #[test]
    fn add_directory() -> Result<()> {
        let mut node = Node::empty("foo.jiji", "");
        node.add_directory("foo".into())?;

        assert_eq!(node.directories.len(), 1, "one directory tracked");
        assert_eq!(node.directories[0].path, "foo", "correct directory path");
        let directory = &node.directories[0];
        let DirectoryChildren::Resolved(children) = &directory.children else {
            panic!("directory children should be resolved");
        };
        assert!(children.is_empty(), "directory has no files");
        assert!(!directory.is_dirty(), "directory not dirty on creation");
        assert!(!node.is_dirty(), "node not dirty on creation");

        Ok(())
    }

    #[test]
    fn add_directory_twice_is_noop() -> Result<()> {
        let mut node = Node::empty("foo.jiji", "");
        node.add_directory("foo".into())?;
        let snapshot = node.clone();

        node.add_directory("foo".into())?;

        assert_eq!(node, snapshot, "node should be unchanged");

        Ok(())
    }

    #[test]
    fn add_nested_directory_conflict() -> Result<()> {
        let mut node = Node::empty("foo.jiji", "");
        node.add_directory("foo".into())?;

        assert_eq!(node.directories.len(), 1, "one directory tracked first");

        let result = node.add_directory("foo/bar".into());

        assert!(
            result.is_err(),
            "adding nested directory should fail due to conflict"
        );

        Ok(())
    }

    #[test]
    fn add_parent_directory_conflict() -> Result<()> {
        let mut node = Node::empty("foo.jiji", "");
        node.add_directory("foo/bar".into())?;

        assert_eq!(node.directories.len(), 1, "one directory tracked first");

        let result = node.add_directory("foo".into());

        assert!(
            result.is_err(),
            "adding parent directory should fail due to conflict"
        );

        Ok(())
    }

    #[test]
    fn add_file_then_directory_conflict() -> Result<()> {
        let mut node = Node::empty("foo.jiji", "");
        node.add_file(Utf8Path::new("foo/data.txt"), Hash::from_bytes([1u8; 32]))?;

        assert_eq!(node.files.len(), 1, "file added successfully first");
        assert_eq!(node.directories.len(), 0, "no directories yet");

        let result = node.add_directory("foo".into());

        assert!(
            result.is_err(),
            "adding parent directory after file should fail"
        );

        Ok(())
    }

    #[test]
    fn persist_clean_node() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let hash = Hash::from_bytes([1u8; 32]);

        let mut node = Node {
            path: "foo.txt.jiji".into(),
            base: "".into(),
            files: vec![File {
                path: "foo.txt".into(),
                hash,
                status: FileStatus::Unknown,
            }],
            directories: Vec::new(),
        };

        node.persist_to_disk(&repo)?;

        assert!(!node.is_dirty(), "node should remain not dirty");
        assert!(
            !Utf8Path::new("foo.txt.jiji").exists(),
            "reference file should not be created for clean node"
        );

        Ok(())
    }

    #[test]
    fn persist_node_with_pending_file() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::write("foo.txt", "hello world")?;
        let hash = hash_file("foo.txt")?;

        let mut node = Node {
            path: "foo.txt.jiji".into(),
            base: "".into(),
            files: vec![File {
                path: "foo.txt".into(),
                hash,
                status: FileStatus::Staged,
            }],
            directories: Vec::new(),
        };
        node.persist_to_disk(&repo)?;

        assert!(!node.is_dirty(), "node should be marked not dirty");
        assert!(
            Utf8Path::new("foo.txt.jiji").exists(),
            "reference file should be created"
        );
        let reference_file = ReferenceFile::read("foo.txt.jiji")?;
        assert_eq!(reference_file.files.len(), 1, "one file tracked");
        assert_eq!(reference_file.files[0].path, "foo.txt", "correct file path");
        assert_eq!(reference_file.files[0].hash, hash, "correct file hash");
        assert!(
            reference_file.directories.is_empty(),
            "no directories tracked"
        );
        let cache_path = repo.cache_path_for(hash);
        assert!(cache_path.exists(), "file should be cached");
        let content = fs::read_to_string(cache_path)?;
        assert_eq!(content, "hello world", "cached file content should match");
        assert!(
            node.files[0].status == FileStatus::Unknown,
            "file status should be updated to clean"
        );
        Ok(())
    }

    #[test]
    fn persist_node_with_pending_file_from_nested_cwd() -> Result<()> {
        let (repo, _tmp, mut guard) = setup_repo()?;

        let nested_dir = repo.root.join("nested/deeper");
        fs::create_dir_all(nested_dir.as_std_path())?;
        fs::write(repo.root.join("foo.txt"), "hello world")?;
        let hash = hash_file(repo.root.join("foo.txt"))?;

        let mut node = Node {
            path: "foo.txt.jiji".into(),
            base: "".into(),
            files: vec![File {
                path: "foo.txt".into(),
                hash,
                status: FileStatus::Staged,
            }],
            directories: Vec::new(),
        };

        guard.change_to(&nested_dir)?;
        node.persist_to_disk(&repo)?;

        let reference_file = ReferenceFile::read(repo.root.join("foo.txt.jiji"))?;
        assert_eq!(reference_file.files.len(), 1, "one file tracked");
        assert_eq!(reference_file.files[0].path, "foo.txt", "correct file path");
        assert_eq!(reference_file.files[0].hash, hash, "correct file hash");

        let cache_path = repo.cache_path_for(hash);
        assert!(cache_path.exists(), "file should be cached");
        let content = fs::read_to_string(cache_path)?;
        assert_eq!(content, "hello world", "cached file content should match");
        assert_eq!(
            node.files[0].status,
            FileStatus::Unknown,
            "file should be clean"
        );

        Ok(())
    }

    #[test]
    fn persist_node_with_pending_directory() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        fs::create_dir_all("dir")?;
        fs::write("dir/foo.txt", "foo content")?;

        let hash_foo = hash_file("dir/foo.txt")?;

        let mut node = Node {
            path: "dir.jiji".into(),
            base: "".into(),
            files: Vec::new(),
            directories: vec![Directory {
                path: "dir".into(),
                hash: None,
                children: DirectoryChildren::Resolved(vec![File {
                    path: "foo.txt".into(),
                    hash: hash_foo,
                    status: FileStatus::Staged,
                }]),
            }],
        };

        node.persist_to_disk(&repo)?;

        assert!(!node.is_dirty(), "node should be marked not dirty");

        assert!(
            Utf8Path::new("dir.jiji").exists(),
            "reference file should be created"
        );
        let reference_file = ReferenceFile::read("dir.jiji")?;
        assert_eq!(reference_file.files.len(), 0, "no files tracked");
        assert_eq!(reference_file.directories.len(), 1, "one directory tracked");

        let cache_path_dir = repo.cache_path_for(reference_file.directories[0].hash);
        assert!(
            cache_path_dir.exists(),
            "directory reference should be cached"
        );
        let dir_reference = ReferenceFile::read(cache_path_dir)?;
        assert_eq!(dir_reference.files.len(), 1, "one file in cached directory");
        assert_eq!(
            dir_reference.files[0].path, "foo.txt",
            "correct file path in cached directory"
        );
        assert_eq!(
            dir_reference.files[0].hash, hash_foo,
            "correct file hash in cached directory"
        );
        assert!(
            dir_reference.directories.is_empty(),
            "no subdirectories in cached directory"
        );

        let cache_path_foo = repo.cache_path_for(hash_foo);
        assert!(cache_path_foo.exists(), "foo.txt should be cached");
        let content_foo = fs::read_to_string(cache_path_foo)?;
        assert_eq!(
            content_foo, "foo content",
            "cached foo.txt content should match"
        );

        Ok(())
    }

    #[test]
    fn persist_node_with_pending_directory_from_nested_cwd() -> Result<()> {
        let (repo, _tmp, mut guard) = setup_repo()?;

        let nested_dir = repo.root.join("nested/deeper");
        fs::create_dir_all(nested_dir.as_std_path())?;
        fs::create_dir_all(repo.root.join("dir").as_std_path())?;
        fs::write(repo.root.join("dir/foo.txt"), "foo content")?;

        let hash_foo = hash_file(repo.root.join("dir/foo.txt"))?;

        let mut node = Node {
            path: "dir.jiji".into(),
            base: "".into(),
            files: Vec::new(),
            directories: vec![Directory {
                path: "dir".into(),
                hash: None,
                children: DirectoryChildren::Resolved(vec![File {
                    path: "foo.txt".into(),
                    hash: hash_foo,
                    status: FileStatus::Staged,
                }]),
            }],
        };

        guard.change_to(&nested_dir)?;
        node.persist_to_disk(&repo)?;

        let reference_file = ReferenceFile::read(repo.root.join("dir.jiji"))?;
        assert_eq!(reference_file.files.len(), 0, "no files tracked");
        assert_eq!(reference_file.directories.len(), 1, "one directory tracked");

        let cache_path_dir = repo.cache_path_for(reference_file.directories[0].hash);
        assert!(
            cache_path_dir.exists(),
            "directory reference should be cached"
        );
        let dir_reference = ReferenceFile::read(cache_path_dir)?;
        assert_eq!(dir_reference.files.len(), 1, "one file in cached directory");
        assert_eq!(dir_reference.files[0].path, "foo.txt");
        assert_eq!(dir_reference.files[0].hash, hash_foo);

        let cache_path_foo = repo.cache_path_for(hash_foo);
        assert!(cache_path_foo.exists(), "foo.txt should be cached");
        assert_eq!(fs::read_to_string(cache_path_foo)?, "foo content");
        assert_eq!(
            node.directories[0].children,
            DirectoryChildren::Resolved(vec![File {
                path: "foo.txt".into(),
                hash: hash_foo,
                status: FileStatus::Unknown,
            }]),
            "directory child should be marked clean"
        );

        Ok(())
    }
}
