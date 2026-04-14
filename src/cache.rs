use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::eyre::{Context as _, ContextCompat as _, Result};
use std::{
    fs::{self, File},
    io::Write as _,
    str::FromStr as _,
};
use tracing::debug;

use crate::{
    hashing::{hash_bytes, Hash},
    reference_file::ReferenceFile,
    JijiRepository,
};

pub(crate) fn cache_relative_path_for(hash: Hash) -> Utf8PathBuf {
    let hex = hash.to_hex();
    let (prefix, suffix) = hex.as_str().split_at(2);
    Utf8PathBuf::from(prefix).join(suffix)
}

pub(crate) fn parse_cache_relative_hash(path: &Utf8Path) -> Result<Hash> {
    let mut components = path.components();
    let prefix = components
        .next()
        .wrap_err_with(|| format!("cache path {path} is missing hash prefix"))?
        .as_str();
    let suffix = components
        .next()
        .wrap_err_with(|| format!("cache path {path} is missing hash suffix"))?
        .as_str();
    if components.next().is_some() {
        color_eyre::eyre::bail!("cache path {path} does not match expected layout");
    }

    Hash::from_str(&format!("{prefix}{suffix}"))
        .wrap_err_with(|| format!("cache path {path} does not contain a valid hash"))
}

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
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            self.root.join(path)
        };
        debug!(%path, %hash, %cache_path, "caching file ...");
        fs::copy(&path, &cache_path)
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
        self.cache_root().join(cache_relative_path_for(hash))
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

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8Path;
    use color_eyre::Result;

    use crate::{
        cache::{cache_relative_path_for, parse_cache_relative_hash},
        hashing::{hash_file, Hash},
        test_utils::setup_repo,
    };

    #[test]
    fn cache_layout_helpers_round_trip_hash() -> Result<()> {
        let hash = Hash::from_bytes([0xabu8; 32]);

        let relative_path = cache_relative_path_for(hash);

        assert_eq!(relative_path, Utf8Path::new("ab").join(&hash.to_hex()[2..]));
        assert_eq!(parse_cache_relative_hash(&relative_path)?, hash);

        Ok(())
    }

    #[test]
    fn cache_file_uses_repository_root_from_nested_cwd() -> Result<()> {
        let (repo, _tmp, mut guard) = setup_repo()?;

        let nested_dir = repo.root.join("nested/deeper");
        fs::create_dir_all(nested_dir.as_std_path())?;
        fs::write(repo.root.join("file.txt"), "hello world")?;
        let hash = hash_file(repo.root.join("file.txt"))?;

        guard.change_to(&nested_dir)?;
        repo.cache_file(hash, "file.txt")?;

        let cache_path = repo.cache_path_for(hash);
        assert!(cache_path.exists(), "cache entry should be created");
        assert_eq!(fs::read_to_string(cache_path)?, "hello world");

        Ok(())
    }
}
