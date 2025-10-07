use std::{fs::File, str::FromStr as _};

use camino::Utf8Path;
use color_eyre::{eyre::Result, eyre::WrapErr as _};
use serde::Deserialize as _;

pub type Hash = blake3::Hash;

pub fn hash_file(path: impl AsRef<Utf8Path>) -> Result<Hash> {
    let path = path.as_ref();

    let file = File::open(path).wrap_err("failed to open file")?;

    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(file).wrap_err("failed to read file")?;

    Ok(hasher.finalize())
}

pub fn hash_bytes(bytes: &[u8]) -> Hash {
    blake3::hash(bytes)
}

pub fn serialize_hash_as_hex<S>(hash: &Hash, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let hex = hash.to_hex();
    serializer.serialize_str(&hex)
}

pub fn deserialize_hash_from_hex<'de, D>(deserializer: D) -> Result<Hash, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let hex = String::deserialize(deserializer)?;
    blake3::Hash::from_str(&hex).map_err(serde::de::Error::custom)
}
