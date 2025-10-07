use crate::JijiRepository;
use camino::Utf8Path;
use color_eyre::Result;
use std::env::set_current_dir;
use tempfile::TempDir;

pub fn setup_repo() -> Result<(JijiRepository, TempDir)> {
    let tmp = tempfile::tempdir()?;
    let repo_path = <&Utf8Path>::try_from(tmp.path())?;
    let repo = JijiRepository::init(repo_path)?;
    set_current_dir(repo_path)?;
    Ok((repo, tmp))
}
