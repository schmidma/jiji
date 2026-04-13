use std::{
    fmt::{self, Debug, Display},
    fs::{self, File},
    io::Write as _,
};

use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{eyre::Context as _, Result};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::hashing::{deserialize_hash_from_hex, serialize_hash_as_hex, Hash};

/// A reference to a file or directory with its associated hash.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reference {
    pub path: Utf8PathBuf,
    #[serde(
        serialize_with = "serialize_hash_as_hex",
        deserialize_with = "deserialize_hash_from_hex"
    )]
    pub hash: Hash,
}

impl Reference {
    /// Creates a new `Reference` from a given path and hash.
    pub const fn new(path: Utf8PathBuf, hash: Hash) -> Self {
        Self { path, hash }
    }

    /// Creates a new `Reference` using a path relative to a given base.
    ///
    /// If the path cannot be made relative to the base, an error is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use camino::Utf8Path;
    /// use jiji::Reference;
    ///
    /// let base = Utf8Path::new("/home/user");
    /// let target = Utf8Path::new("/home/user/project/file.txt");
    /// let hash = blake3::hash(b"file content");
    /// let reference = Reference::from_path_with_base(target, base, hash).unwrap();
    /// assert!(reference.path == "project/file.txt");
    /// ```
    pub fn from_path_with_base(
        path: impl AsRef<Utf8Path> + Debug,
        base: impl AsRef<Utf8Path> + Debug,
        hash: Hash,
    ) -> Result<Self> {
        let relative_path = path
            .as_ref()
            .strip_prefix(base.as_ref())
            .wrap_err_with(|| {
                format!(
                    "failed to make path '{path}' relative to base '{base}'",
                    path = path.as_ref(),
                    base = base.as_ref()
                )
            })?;
        Ok(Self::new(relative_path.to_owned(), hash))
    }
}

impl Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{path}` ({hash})", path = self.path, hash = self.hash)
    }
}

/// A container for references to files and directories.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReferenceFile {
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub files: Vec<Reference>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub directories: Vec<Reference>,
}

impl ReferenceFile {
    /// Creates an empty `ReferenceFile`.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Adds a file reference to the `ReferenceFile`.
    pub fn add_file(&mut self, file: Reference) -> &mut Self {
        self.files.push(file);
        self
    }

    /// Adds a directory reference to the `ReferenceFile`.
    pub fn add_directory(&mut self, directory: Reference) -> &mut Self {
        self.directories.push(directory);
        self
    }

    /// Reads a `ReferenceFile` from a TOML file at the specified path.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened, read, or parsed as TOML.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use camino::Utf8Path;
    /// use jiji::ReferenceFile;
    ///
    /// let reference_file = ReferenceFile::read(Utf8Path::new("references.toml")).unwrap();
    /// ```
    pub fn read(path: impl AsRef<Utf8Path> + Debug) -> Result<Self> {
        let path = path.as_ref();
        debug!("reading reference file from {path}");

        let content = fs::read_to_string(path).wrap_err("failed to read content")?;
        let reference = toml::from_str(&content).wrap_err("failed to parse as TOML")?;
        Ok(reference)
    }

    // TODO: think about naming conflict with serialize method from the trait
    /// Serializes the `ReferenceFile` to a TOML string.
    pub fn serialize(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(&self)
    }

    /// Writes the `ReferenceFile` to a file at the specified path in TOML format.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be created or written.
    pub fn write(&self, path: impl AsRef<Utf8Path> + Debug) -> Result<()> {
        let path = path.as_ref();
        debug!("writing reference to {path}");

        let serialized = self
            .serialize()
            .wrap_err("failed to serialize reference file")?;
        let mut file =
            File::create(path).wrap_err_with(|| format!("failed to create file: {path}"))?;
        file.write_all(serialized.as_bytes())
            .wrap_err("failed to write reference to file")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8Path;
    use color_eyre::Result;

    const DUMMY_HASH: Hash = Hash::from_bytes([1u8; 32]);

    #[test]
    fn reference_new() {
        let reference = Reference::new("foo.txt".into(), DUMMY_HASH);
        assert_eq!(reference.path, "foo.txt", "path should match input");
        assert_eq!(reference.hash, DUMMY_HASH, "hash should match input");
    }

    #[test]
    fn reference_from_path_with_base_relative() -> Result<()> {
        let base = Utf8Path::new("/home/user/project");
        let target = Utf8Path::new("/home/user/project/src/main.rs");

        let reference = Reference::from_path_with_base(target, base, DUMMY_HASH)?;
        assert_eq!(
            reference.path,
            Utf8Path::new("src/main.rs"),
            "path should be relative to base"
        );
        assert_eq!(reference.hash, DUMMY_HASH, "hash should be preserved");

        Ok(())
    }

    #[test]
    fn reference_from_path_with_base_outside_base_fails() {
        let base = Utf8Path::new("/home/user/project");
        let target = Utf8Path::new("/home/user/other/file.txt");

        let res = Reference::from_path_with_base(target, base, DUMMY_HASH);
        assert!(res.is_err(), "should error if path cannot be made relative");
    }

    #[test]
    fn reference_file_empty() {
        let rf = ReferenceFile::empty();
        assert!(rf.files.is_empty(), "files should be empty");
        assert!(rf.directories.is_empty(), "directories should be empty");
    }

    #[test]
    fn reference_file_add_file_and_directory() {
        let mut rf = ReferenceFile::empty();
        let file_ref = Reference::new("a.txt".into(), DUMMY_HASH);
        let dir_ref = Reference::new("data".into(), DUMMY_HASH);

        rf.add_file(file_ref.clone());
        rf.add_directory(dir_ref.clone());

        assert_eq!(rf.files.len(), 1, "should contain one file");
        assert_eq!(rf.files[0], file_ref, "file reference should match");
        assert_eq!(rf.directories.len(), 1, "should contain one directory");
        assert_eq!(
            rf.directories[0], dir_ref,
            "directory reference should match"
        );
    }

    #[test]
    fn reference_file_read_write_roundtrip() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = Utf8PathBuf::try_from(temp_dir.path().join("refs.toml")).unwrap();

        let file_ref = Reference::new("file.txt".into(), DUMMY_HASH);
        let dir_ref = Reference::new("dir".into(), DUMMY_HASH);

        let mut rf = ReferenceFile::empty();
        rf.add_file(file_ref.clone()).add_directory(dir_ref.clone());

        rf.write(&path)?;
        let read_rf = ReferenceFile::read(&path)?;

        assert_eq!(read_rf.files.len(), 1, "should read back one file");
        assert_eq!(
            read_rf.files[0], file_ref,
            "file reference should match after read"
        );
        assert_eq!(
            read_rf.directories.len(),
            1,
            "should read back one directory"
        );
        assert_eq!(
            read_rf.directories[0], dir_ref,
            "directory reference should match after read"
        );

        Ok(())
    }

    #[test]
    fn reference_file_serialize_to_toml() -> Result<()> {
        let mut rf = ReferenceFile::empty();
        rf.add_file(Reference::new("file.txt".into(), DUMMY_HASH))
            .add_file(Reference::new("foo.txt".into(), DUMMY_HASH));

        let toml_str = rf.serialize()?;
        assert!(
            toml_str.contains(r#"path = "file.txt""#) && toml_str.contains(r#"path = "foo.txt""#),
            "serialized TOML should contain all files"
        );
        assert!(
            toml_str.contains(&DUMMY_HASH.to_string()),
            "serialized TOML should include hash values"
        );
        Ok(())
    }

    #[test]
    fn reference_file_multiple_files_and_dirs_roundtrip() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = Utf8PathBuf::try_from(temp_dir.path().join("refs.toml")).unwrap();

        let mut rf = ReferenceFile::empty();
        for i in 0..3 {
            rf.add_file(Reference::new(format!("f{i}.txt").into(), DUMMY_HASH));
            rf.add_directory(Reference::new(format!("d{i}").into(), DUMMY_HASH));
        }

        rf.write(&path)?;
        let read_rf = ReferenceFile::read(&path)?;

        assert_eq!(read_rf.files.len(), 3, "should preserve all files");
        assert_eq!(
            read_rf.directories.len(),
            3,
            "should preserve all directories"
        );

        for i in 0..3 {
            assert_eq!(read_rf.files[i].path, format!("f{i}.txt"));
            assert_eq!(read_rf.directories[i].path, format!("d{i}"));
        }

        Ok(())
    }
}
