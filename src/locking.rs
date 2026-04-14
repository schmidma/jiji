use std::{
    fs::{File, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
};

use color_eyre::{eyre::Context as _, Result};
use fs2::FileExt;

pub(crate) enum LockMode {
    Read,
    Write,
}

pub(crate) struct RepositoryLock {
    path: PathBuf,
}

pub(crate) struct RepositoryLockGuard {
    file: File,
}

impl RepositoryLock {
    pub(crate) fn new(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            path: path.as_ref().to_path_buf(),
        })
    }

    pub(crate) fn acquire(
        &self,
        mode: LockMode,
        on_wait: impl FnOnce(),
    ) -> Result<RepositoryLockGuard> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.path)
            .wrap_err_with(|| format!("failed to open lock file {}", self.path.display()))?;

        let try_lock = match mode {
            LockMode::Read => FileExt::try_lock_shared(&file),
            LockMode::Write => FileExt::try_lock_exclusive(&file),
        };

        match try_lock {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                on_wait();
                match mode {
                    LockMode::Read => file.lock_shared(),
                    LockMode::Write => file.lock_exclusive(),
                }
                .wrap_err_with(|| format!("failed to acquire lock file {}", self.path.display()))?;
            }
            Err(error) => Err(error)
                .wrap_err_with(|| format!("failed to acquire lock file {}", self.path.display()))?,
        }

        Ok(RepositoryLockGuard { file })
    }
}

impl Drop for RepositoryLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    use color_eyre::Result;
    use tempfile::tempdir;

    use crate::locking::{LockMode, RepositoryLock};

    #[test]
    fn shared_read_locks_can_overlap() -> Result<()> {
        let temp = tempdir()?;
        let lock = RepositoryLock::new(temp.path().join("repo.lock"))?;

        let _first = lock.acquire(LockMode::Read, || {})?;
        let start = Instant::now();
        let _second = lock.acquire(LockMode::Read, || {})?;

        assert!(
            start.elapsed() < Duration::from_millis(100),
            "shared read lock acquisition should not wait for another reader"
        );

        Ok(())
    }

    #[test]
    fn writer_waits_until_reader_releases() -> Result<()> {
        let temp = tempdir()?;
        let lock_path = temp.path().join("repo.lock");
        let reader_lock = RepositoryLock::new(&lock_path)?;
        let writer_lock = RepositoryLock::new(&lock_path)?;
        let reader_guard = reader_lock.acquire(LockMode::Read, || {})?;

        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();

        let writer = thread::spawn(move || -> Result<()> {
            started_tx.send(Instant::now())?;
            let _writer_guard = writer_lock.acquire(LockMode::Write, || {})?;
            finished_tx.send(Instant::now())?;
            Ok(())
        });

        let started_at = started_rx.recv()?;
        thread::sleep(Duration::from_millis(150));
        assert!(
            finished_rx.try_recv().is_err(),
            "writer should stay blocked while reader holds the lock"
        );

        drop(reader_guard);

        let finished_at = finished_rx.recv()?;
        writer.join().expect("writer thread should not panic")?;

        assert!(
            finished_at.duration_since(started_at) >= Duration::from_millis(150),
            "writer should not finish before the reader releases the lock"
        );

        Ok(())
    }

    #[test]
    fn waiting_writer_invokes_on_wait_before_reader_releases() -> Result<()> {
        let temp = tempdir()?;
        let lock_path = temp.path().join("repo.lock");
        let reader_lock = RepositoryLock::new(&lock_path)?;
        let writer_lock = RepositoryLock::new(&lock_path)?;
        let reader_guard = reader_lock.acquire(LockMode::Read, || {})?;

        let (waiting_tx, waiting_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();

        let writer = thread::spawn(move || -> Result<()> {
            let _writer_guard = writer_lock.acquire(LockMode::Write, || {
                waiting_tx.send(()).expect("waiting signal should send");
            })?;
            finished_tx.send(())?;
            Ok(())
        });

        waiting_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("writer should report that it is waiting before reader releases lock");
        assert!(
            finished_rx.try_recv().is_err(),
            "writer should remain blocked until the read lock is released"
        );

        drop(reader_guard);

        finished_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("writer should finish once the reader releases the lock");
        writer.join().expect("writer thread should not panic")?;

        Ok(())
    }
}
