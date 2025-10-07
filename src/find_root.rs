use camino::{absolute_utf8, Utf8Path, Utf8PathBuf};
use color_eyre::eyre::{Context as _, ContextCompat as _, Result};
use pathdiff::diff_utf8_paths;
use std::fs;
use tracing::debug;

use crate::JijiRepository;

impl JijiRepository {
    /// Search for a `.jiji` directory in the ancestors of `start` and return the path to the directory
    /// containing it.
    pub fn find_upwards_from(start: impl AsRef<Utf8Path>) -> Result<Self> {
        let start = absolute_utf8(start.as_ref()).wrap_err("failed to get absolute path")?;

        let root = start
            .ancestors()
            .find(|ancestor| is_jiji_root_dir(ancestor))
            .wrap_err("searched all ancestors but could not find repository root")?;

        let mut relative_root = diff_utf8_paths(root, &start).expect("paths have common prefix");
        if relative_root.as_str().is_empty() {
            relative_root = Utf8PathBuf::from(".");
        }

        Self::new(relative_root).wrap_err("failed to create repository")
    }
}

fn is_jiji_root_dir(ancestor: &Utf8Path) -> bool {
    let candidate = ancestor.join(".jiji");
    match fs::metadata(candidate.as_std_path()) {
        Ok(metadata) => metadata.is_dir(),
        Err(error) => {
            debug!("failed to get metadata for {candidate}: {error:#}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn find_root_in_current_dir() -> Result<()> {
        let tmp = tempdir()?;
        let root = <&Utf8Path>::try_from(tmp.path())?;
        fs::create_dir_all(root.join(".jiji"))?;

        let repo = JijiRepository::find_upwards_from(root)?;

        assert_eq!(repo.root, ".", "root should be '.' for current dir");

        Ok(())
    }

    #[test]
    fn find_root_in_parent_dir() -> Result<()> {
        let tmp = tempdir()?;
        let root = <&Utf8Path>::try_from(tmp.path())?;
        fs::create_dir_all(root.join(".jiji"))?;

        let nested = root.join("a/b/c");
        fs::create_dir_all(&nested)?;

        let repo = JijiRepository::find_upwards_from(&nested)?;

        assert_eq!(
            repo.root, "../../..",
            "root should be relative path to start"
        );

        Ok(())
    }

    #[test]
    fn find_root_no_jiji_error() -> Result<()> {
        let tmp = tempdir()?;
        let root = <&Utf8Path>::try_from(tmp.path())?;

        let res = JijiRepository::find_upwards_from(root);

        assert!(
            res.is_err(),
            "should error if no .jiji directory is present in ancestors"
        );

        Ok(())
    }
}
