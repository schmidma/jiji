mod path;
mod sftp;

use camino::Utf8Path;
use color_eyre::{
    eyre::{bail, Context, ContextCompat},
    Result,
};
use tracing::warn;

use crate::{
    configuration::{Configuration, StorageConfiguration},
    hashing::Hash,
    index::DirectoryChildren,
    reference_file::Reference,
    storage::path::PathStorage,
    JijiRepository,
};

pub trait Storage {
    /// Store `file` (path to a file) addressed by `hash`
    fn store(&self, hash: Hash, object: impl AsRef<Utf8Path>) -> Result<()>;

    /// Retrieve object addressed by `hash` into `destination`.
    fn retrieve(&self, hash: Hash, destination: impl AsRef<Utf8Path>) -> Result<()>;
}

pub enum StorageBackend {
    Path(PathStorage),
    Sftp(sftp::SftpStorage),
}

impl Storage for StorageBackend {
    fn store(&self, hash: Hash, object: impl AsRef<Utf8Path>) -> Result<()> {
        match self {
            StorageBackend::Path(s) => s.store(hash, object),
            StorageBackend::Sftp(s) => s.store(hash, object),
        }
    }

    fn retrieve(&self, hash: Hash, destination: impl AsRef<Utf8Path>) -> Result<()> {
        match self {
            StorageBackend::Path(s) => s.retrieve(hash, destination),
            StorageBackend::Sftp(s) => s.retrieve(hash, destination),
        }
    }
}

impl JijiRepository {
    fn init_storage_backend_from_config(
        &self,
        configuration: &Configuration,
        storage_name: &str,
    ) -> Result<StorageBackend> {
        let config = configuration.storages.get(storage_name).wrap_err_with(|| {
            format!("storage '{storage_name}' not found in repository configuration")
        })?;

        match config {
            StorageConfiguration::Path { location } => {
                Ok(StorageBackend::Path(PathStorage::new(location)))
            }
            StorageConfiguration::Sftp(configuration) => Ok(StorageBackend::Sftp(
                sftp::SftpStorage::connect(configuration).wrap_err_with(|| {
                    format!("failed to connect to SFTP storage '{storage_name}'")
                })?,
            )),
        }
    }

    pub fn push(&self, storage_name: &str) -> Result<()> {
        self.with_write_lock("push", |repository| {
            let configuration = repository.load_configuration_fresh()?;
            let storage = repository
                .init_storage_backend_from_config(&configuration, storage_name)
                .wrap_err_with(|| {
                    format!("failed to initialize storage backend '{storage_name}'")
                })?;

            let index = repository.index().wrap_err("failed to index repository")?;

            for node in index.iter_nodes() {
                for file in &node.files {
                    let cached_path = repository.cache_path_for(file.hash);
                    if !cached_path.exists() {
                        bail!("object {} not found in cache at {cached_path}", file.hash);
                    }
                    storage.store(file.hash, &cached_path).wrap_err_with(|| {
                        format!(
                            "failed to store object '{}' to storage '{}'",
                            file.hash, storage_name
                        )
                    })?;
                }

                for directory in &node.directories {
                    let Some(hash) = directory.hash else {
                        warn!(
                            "directory '{}' has no hash, skipping",
                            node.base.join(&directory.path)
                        );
                        continue;
                    };

                    let dir_cached_path = repository.cache_path_for(hash);
                    if !dir_cached_path.exists() {
                        bail!(
                            "directory manifest {} not in cache at {dir_cached_path}",
                            hash
                        );
                    }

                    storage.store(hash, &dir_cached_path).wrap_err_with(|| {
                        format!(
                            "failed to store directory '{}' to storage '{}'",
                            node.base.join(&directory.path),
                            storage_name
                        )
                    })?;

                    let DirectoryChildren::Resolved(children) = &directory.children else {
                        warn!(
                            "directory '{}' has no children information, skipping children",
                            node.base.join(&directory.path)
                        );
                        continue;
                    };

                    for child in children {
                        let child_cached = repository.cache_path_for(child.hash);
                        if !child_cached.exists() {
                            bail!("child object {} not in cache at {child_cached}", child.hash);
                        }
                        storage.store(child.hash, &child_cached).wrap_err_with(|| {
                            format!(
                                "failed to store child object '{}' to storage '{}'",
                                child.hash, storage_name
                            )
                        })?;
                    }
                }
            }

            Ok(())
        })
    }

    pub fn fetch(&self, storage_name: &str) -> Result<()> {
        self.with_write_lock("fetch", |repository| {
            let configuration = repository.load_configuration_fresh()?;
            let storage = repository
                .init_storage_backend_from_config(&configuration, storage_name)
                .wrap_err_with(|| {
                    format!("failed to initialize storage backend '{storage_name}'")
                })?;
            let mut index = repository.index().wrap_err("failed to index repository")?;

            for node in index.iter_nodes_mut() {
                for file in &node.files {
                    let workspace_path = node.base.join(&file.path);
                    let cached = repository.cache_path_for(file.hash);
                    if cached.exists() {
                        continue;
                    }
                    storage.retrieve(file.hash, &cached).wrap_err_with(|| {
                        format!(
                            "failed to fetch file '{}' from storage '{}'",
                            workspace_path, storage_name
                        )
                    })?;
                }

                for directory in &mut node.directories {
                    let Some(hash) = directory.hash else {
                        warn!(
                            "directory '{}' has no hash, skipping",
                            node.base.join(&directory.path)
                        );
                        continue;
                    };

                    let dir_cached = repository.cache_path_for(hash);
                    if !dir_cached.exists() {
                        storage.retrieve(hash, &dir_cached).wrap_err_with(|| {
                            format!(
                                "failed to fetch directory manifest '{}' from storage '{}'",
                                node.base.join(&directory.path),
                                storage_name
                            )
                        })?;
                    }

                    let children = match &directory.children {
                        DirectoryChildren::Resolved(children) => children.clone(),
                        DirectoryChildren::NotInCache => {
                            let new_directory = repository
                                .index_directory(Reference::new(directory.path.clone(), hash))
                                .wrap_err_with(|| {
                                    format!(
                                        "failed to index directory '{}' in cache",
                                        node.base.join(&directory.path)
                                    )
                                })?;
                            *directory = new_directory;
                            match &directory.children {
                                DirectoryChildren::Resolved(children) => children.clone(),
                                DirectoryChildren::NotInCache => bail!(
                                    "directory '{}' has no children after indexing",
                                    directory.path
                                ),
                            }
                        }
                    };

                    for child in children {
                        let child_cached = repository.cache_path_for(child.hash);
                        if child_cached.exists() {
                            continue;
                        }
                        storage
                            .retrieve(child.hash, &child_cached)
                            .wrap_err_with(|| {
                                format!(
                                    "failed to fetch child '{}' for directory '{}' from storage '{}'",
                                    child.path,
                                    node.base.join(&directory.path),
                                    storage_name
                                )
                            })?;
                    }
                }
            }

            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, thread, time::Duration};

    use color_eyre::Result;

    use crate::{locking::LockMode, test_utils::setup_repo};

    #[test]
    fn push_blocks_while_read_lock_held() -> Result<()> {
        let (repo, _tmp, _guard) = setup_repo()?;
        let storage_root = repo.workspace_root().join("storage");
        std::fs::create_dir_all(storage_root.as_std_path())?;
        repo.add_storage("local", &format!("file://{storage_root}"))?;

        let read_guard = repo.repository_lock()?.acquire(LockMode::Read, || {})?;
        let (finished_tx, finished_rx) = mpsc::channel();

        thread::scope(|scope| {
            scope.spawn(|| {
                finished_tx
                    .send(repo.push("local"))
                    .expect("push result should send");
            });

            thread::sleep(Duration::from_millis(150));
            assert!(
                finished_rx.try_recv().is_err(),
                "push should stay blocked while a read lock is held"
            );

            drop(read_guard);

            finished_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("push should complete after the read lock is released")
                .expect("push should succeed once the lock is available");
        });

        Ok(())
    }
}
