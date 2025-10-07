use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::eyre::{Context as _, Result};
use std::{
    fs::{self, File},
    io::Write as _,
};
use tracing::debug;

use crate::{
    hashing::{hash_bytes, Hash},
    reference_file::ReferenceFile,
    JijiRepository,
};

impl JijiRepository {
    pub fn cache_file(&self, hash: Hash, path: impl AsRef<Utf8Path>) -> Result<()> {
        let cache_path = self.cache_path_for(hash);

        if cache_path.exists() {
            debug!(%hash, "file already in cache");
            return Ok(());
        }

        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)
                .wrap_err_with(|| format!("failed to create directories for {parent}"))?;
        }

        let path = path.as_ref();
        debug!(%path, %hash, %cache_path, "caching file ...");
        fs::copy(path, &cache_path)
            .with_context(|| format!("failed to copy {path} to {cache_path}"))?;

        Ok(())
    }

    pub fn cache_reference_file(&self, reference_file: &ReferenceFile) -> Result<Hash> {
        let serialized = reference_file
            .serialize()
            .wrap_err("failed to serialize reference file")?;
        let hash = hash_bytes(serialized.as_bytes());
        self.cache_data(hash, serialized.as_bytes())
            .wrap_err("failed to cache reference file for directory entries")?;

        Ok(hash)
    }

    pub fn cache_path_for(&self, hash: Hash) -> Utf8PathBuf {
        let hex = hash.to_hex();
        let (prefix, suffix) = hex.as_str().split_at(2);
        self.cache_root().join(prefix).join(suffix)
    }

    fn cache_data(&self, hash: Hash, data: &[u8]) -> Result<()> {
        let cache_path = self.cache_path_for(hash);

        if cache_path.exists() {
            debug!(%hash, "file already in cache");
            return Ok(());
        }

        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)
                .wrap_err_with(|| format!("failed to create directories for {parent}"))?;
        }

        debug!(%hash, %cache_path, "caching data ...");

        let mut file = File::create(&cache_path)
            .wrap_err_with(|| format!("failed to create file: {cache_path}"))?;
        file.write_all(data)
            .wrap_err("failed to write reference to file")?;

        Ok(())
    }
}
