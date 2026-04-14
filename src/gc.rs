use std::{collections::HashSet, fs};

use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    eyre::{bail, Context as _, ContextCompat as _},
    Result,
};
use walkdir::WalkDir;

use crate::{
    cache::parse_cache_relative_hash, hashing::Hash, index::DirectoryChildren, JijiRepository,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GarbageCollectionReport {
    pub reachable_objects: usize,
    pub unreferenced_objects: usize,
    pub removed_objects: usize,
}

impl JijiRepository {
    pub fn gc(&self, dry_run: bool) -> Result<GarbageCollectionReport> {
        let reachable_hashes = self.collect_live_cache_hashes()?;
        let cache_objects = self.collect_cache_objects()?;

        let mut removed_objects = 0;
        let mut unreferenced_objects = 0;

        for (hash, path) in cache_objects {
            if reachable_hashes.contains(&hash) {
                continue;
            }

            unreferenced_objects += 1;
            if dry_run {
                continue;
            }

            fs::remove_file(&path)
                .wrap_err_with(|| format!("failed to remove cache object at {path}"))?;
            removed_objects += 1;
        }

        Ok(GarbageCollectionReport {
            reachable_objects: reachable_hashes.len(),
            unreferenced_objects,
            removed_objects,
        })
    }

    fn collect_live_cache_hashes(&self) -> Result<HashSet<Hash>> {
        let index = self.index().wrap_err("failed to index repository")?;
        let mut live_hashes = HashSet::new();

        for node in index.iter_nodes() {
            for file in &node.files {
                let cached_path = self.cache_path_for(file.hash);
                if !cached_path.exists() {
                    bail!(
                        "object {} not found in cache at {cached_path} for tracked file '{}'",
                        file.hash,
                        node.base.join(&file.path)
                    );
                }
                live_hashes.insert(file.hash);
            }

            for directory in &node.directories {
                let hash = directory.hash.wrap_err_with(|| {
                    format!(
                        "tracked directory '{}' is missing its manifest hash",
                        node.base.join(&directory.path)
                    )
                })?;
                let manifest_path = self.cache_path_for(hash);
                if !manifest_path.exists() {
                    bail!(
                        "directory manifest {} not found in cache at {manifest_path} for tracked directory '{}'",
                        hash,
                        node.base.join(&directory.path)
                    );
                }
                live_hashes.insert(hash);

                let DirectoryChildren::Resolved(children) = &directory.children else {
                    bail!(
                        "directory manifest {} not found in cache at {manifest_path} for tracked directory '{}'",
                        hash,
                        node.base.join(&directory.path)
                    );
                };

                for child in children {
                    let cached_path = self.cache_path_for(child.hash);
                    if !cached_path.exists() {
                        bail!(
                            "object {} not found in cache at {cached_path} for tracked file '{}'",
                            child.hash,
                            node.base.join(&directory.path).join(&child.path)
                        );
                    }
                    live_hashes.insert(child.hash);
                }
            }
        }

        Ok(live_hashes)
    }

    fn collect_cache_objects(&self) -> Result<Vec<(Hash, Utf8PathBuf)>> {
        WalkDir::new(self.cache_root())
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| {
                let path = Utf8PathBuf::try_from(entry.into_path())
                    .wrap_err("cache path is not valid UTF-8")?;
                let hash = self.parse_cache_hash(&path)?;
                Ok((hash, path))
            })
            .collect()
    }

    fn parse_cache_hash(&self, path: &Utf8Path) -> Result<Hash> {
        let relative_path = path
            .strip_prefix(self.cache_root())
            .wrap_err_with(|| format!("cache path {path} is not under {}", self.cache_root()))?;
        parse_cache_relative_hash(relative_path)
            .wrap_err_with(|| format!("failed to parse cache hash from {path}"))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8Path;
    use color_eyre::{eyre::ContextCompat as _, Result};

    use crate::{
        hashing::{hash_bytes, hash_file, Hash},
        test_utils::setup_repo,
        JijiRepository, Reference, ReferenceFile,
    };

    #[test]
    fn gc_dry_run_reports_unreferenced_objects_without_removing_them() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let tracked_hash = write_tracked_file(&repo, "tracked.txt", b"tracked")?;
        let unused_hash = write_cached_object(&repo, b"unused")?;

        let report = repo.gc(true)?;

        assert_eq!(report.reachable_objects, 1);
        assert_eq!(report.unreferenced_objects, 1);
        assert_eq!(report.removed_objects, 0);
        assert!(
            repo.cache_path_for(tracked_hash).exists(),
            "tracked cache object should remain in cache"
        );
        assert!(
            repo.cache_path_for(unused_hash).exists(),
            "dry run should not remove unreferenced cache objects"
        );

        Ok(())
    }

    #[test]
    fn gc_removes_unreferenced_objects() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let tracked_hash = write_tracked_file(&repo, "tracked.txt", b"tracked")?;
        let unused_hash = write_cached_object(&repo, b"unused")?;

        let report = repo.gc(false)?;

        assert_eq!(report.reachable_objects, 1);
        assert_eq!(report.unreferenced_objects, 1);
        assert_eq!(report.removed_objects, 1);
        assert!(
            repo.cache_path_for(tracked_hash).exists(),
            "reachable cache object should not be removed"
        );
        assert!(
            !repo.cache_path_for(unused_hash).exists(),
            "gc should remove unreferenced cache objects"
        );

        Ok(())
    }

    #[test]
    fn gc_errors_when_tracked_file_object_is_missing() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        ReferenceFile::empty()
            .add_file(Reference::new(
                "tracked.txt".into(),
                Hash::from_bytes([7; 32]),
            ))
            .write(repo.root.join("tracked.txt.jiji"))?;

        let error = repo.gc(true).unwrap_err().to_string();

        assert!(
            error.contains("object") && error.contains("tracked.txt"),
            "error should mention the missing tracked file object: {error}"
        );

        Ok(())
    }

    #[test]
    fn gc_errors_when_tracked_directory_manifest_is_missing() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        ReferenceFile::empty()
            .add_directory(Reference::new("data".into(), Hash::from_bytes([9; 32])))
            .write(repo.root.join("data.jiji"))?;

        let error = repo.gc(true).unwrap_err().to_string();

        assert!(
            error.contains("directory manifest") && error.contains("data"),
            "error should mention the missing tracked directory manifest: {error}"
        );

        Ok(())
    }

    #[test]
    fn gc_malformed_cache_path_error_mentions_full_cache_path() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;

        let malformed_path = repo.cache_root().join("ab/not-a-valid-hash");
        let parent = malformed_path
            .parent()
            .wrap_err("malformed cache path should have parent")?;
        fs::create_dir_all(parent)?;
        fs::write(&malformed_path, b"unused")?;

        let error = repo.gc(true).unwrap_err().to_string();

        assert!(
            error.contains(malformed_path.as_str()),
            "error should mention full malformed cache path: {error}"
        );

        Ok(())
    }

    fn write_tracked_file(repo: &JijiRepository, path: &str, contents: &[u8]) -> Result<Hash> {
        fs::write(repo.root.join(path), contents)?;
        let hash = hash_file(repo.root.join(path))?;
        repo.cache_file(hash, path)?;

        let reference_path = Utf8Path::new(path).with_extension("jiji");
        ReferenceFile::empty()
            .add_file(Reference::new(path.into(), hash))
            .write(repo.root.join(reference_path))?;

        Ok(hash)
    }

    fn write_cached_object(repo: &JijiRepository, contents: &[u8]) -> Result<Hash> {
        let hash = hash_bytes(contents);
        let cache_path = repo.cache_path_for(hash);
        let parent = cache_path
            .parent()
            .wrap_err("cache path should have parent")?;
        fs::create_dir_all(parent)?;
        fs::write(cache_path, contents)?;
        Ok(hash)
    }
}
