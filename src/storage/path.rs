use std::{fs, time::SystemTime};

use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    eyre::{bail, Context as _},
    Result,
};
use tracing::debug;

use crate::{hashing::Hash, storage::Storage};

/// A simple filesystem-backed storage backend
#[derive(Debug, Clone)]
pub struct PathStorage {
    location: Utf8PathBuf,
}

impl PathStorage {
    pub fn new(location: impl Into<Utf8PathBuf>) -> Self {
        Self {
            location: location.into(),
        }
    }

    fn object_path_for(&self, hash: Hash) -> Utf8PathBuf {
        let hex = hash.to_hex();
        let (prefix, suffix) = hex.split_at(2);
        self.location.join(prefix).join(suffix)
    }

    fn create_parent_dirs(path: &Utf8Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .wrap_err_with(|| format!("failed to create directories for {parent}"))?;
        }
        Ok(())
    }
}

impl Storage for PathStorage {
    fn store(&self, hash: Hash, object: impl AsRef<Utf8Path>) -> Result<()> {
        let src = object.as_ref();
        let dst = self.object_path_for(hash);

        if dst.exists() {
            debug!("object {hash} already exists in storage at {dst}");
            return Ok(());
        }

        Self::create_parent_dirs(&dst)?;
        atomic_copy(src, &dst)?;
        debug!("stored object {hash} => {dst}");

        Ok(())
    }

    fn retrieve(&self, hash: Hash, destination: impl AsRef<Utf8Path>) -> Result<()> {
        let dst = destination.as_ref();
        let src = self.object_path_for(hash);

        if !src.exists() {
            bail!("object {hash} not found in storage (expected at {src})");
        }

        Self::create_parent_dirs(dst)?;
        atomic_copy(&src, dst)?;

        debug!("retrieved object {} => {dst}", hash);

        Ok(())
    }
}

fn atomic_copy(src: &Utf8Path, dst: &Utf8Path) -> Result<()> {
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp_extension = format!("tmp.{ts}");
    let tmp = dst.with_added_extension(&tmp_extension);
    fs::copy(src, &tmp).with_context(|| format!("failed to copy file from {src} to {tmp}"))?;
    fs::rename(&tmp, dst)
        .with_context(|| format!("failed to rename temporary file {tmp} to {dst}"))?;
    Ok(())
}
