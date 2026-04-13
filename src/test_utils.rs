use crate::JijiRepository;
use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{eyre::Context as _, Result};
use std::{
    env::{current_dir, set_current_dir},
    sync::{Mutex, MutexGuard, OnceLock},
};
use tempfile::TempDir;

pub struct CurrentDirGuard {
    original: Utf8PathBuf,
    _lock: MutexGuard<'static, ()>,
}

impl CurrentDirGuard {
    pub fn set(path: &Utf8Path) -> Result<Self> {
        let lock = current_dir_lock()
            .lock()
            .expect("current dir lock poisoned");
        let original = Utf8PathBuf::try_from(current_dir()?)
            .wrap_err("current directory is not valid utf-8")?;
        set_current_dir(path).wrap_err("failed to set current directory")?;
        Ok(Self {
            original,
            _lock: lock,
        })
    }

    pub fn change_to(&mut self, path: &Utf8Path) -> Result<()> {
        set_current_dir(path).wrap_err("failed to set current directory")
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        set_current_dir(&self.original).expect("failed to restore current directory");
    }
}

fn current_dir_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub fn setup_repo() -> Result<(JijiRepository, TempDir, CurrentDirGuard)> {
    let tmp = tempfile::tempdir()?;
    let repo_path = <&Utf8Path>::try_from(tmp.path())?;
    let repo = JijiRepository::init(repo_path)?;
    let guard = CurrentDirGuard::set(repo_path)?;
    Ok((repo, tmp, guard))
}
