use std::{
    fs::{self, OpenOptions},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use color_eyre::{eyre::eyre, eyre::Context as _, Result};
use fs2::FileExt as _;
use tempfile::tempdir;

fn jiji() -> Command {
    Command::new(env!("CARGO_BIN_EXE_jiji"))
}

fn assert_child_stays_running(child: &mut Child, duration: Duration) -> Result<()> {
    let deadline = Instant::now() + duration;

    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            return Err(eyre!(
                "status exited while another process held the repository lock: {status}"
            ));
        }

        thread::sleep(Duration::from_millis(10));
    }

    Ok(())
}

fn wait_with_timeout(mut child: Child, timeout: Duration) -> Result<Output> {
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        if child.try_wait()?.is_some() {
            return child
                .wait_with_output()
                .wrap_err("failed to collect jiji status output");
        }

        thread::sleep(Duration::from_millis(10));
    }

    child
        .kill()
        .wrap_err("failed to kill timed-out jiji status process")?;
    let output = child
        .wait_with_output()
        .wrap_err("failed to collect output from timed-out jiji status process")?;
    Err(eyre!(
        "timed out waiting for jiji status to finish after releasing the repository lock; status: {}",
        output.status
    ))
}

#[test]
fn cli_wait_message_goes_to_stderr_without_polluting_stdout() -> Result<()> {
    color_eyre::install().ok();

    let tmp = tempdir()?;
    let repo = tmp.path();

    let init_status = jiji()
        .arg("init")
        .arg(repo)
        .status()
        .wrap_err("failed to run jiji init")?;
    assert!(init_status.success(), "jiji init should succeed");

    fs::write(repo.join("file.txt"), "file content")?;

    let add_status = jiji()
        .current_dir(repo)
        .arg("add")
        .arg("file.txt")
        .status()
        .wrap_err("failed to run jiji add")?;
    assert!(add_status.success(), "jiji add should succeed");

    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(repo.join(".jiji/.lock"))
        .wrap_err("failed to open repository lock")?;
    lock_file
        .lock_exclusive()
        .wrap_err("failed to acquire exclusive repository lock")?;

    let mut child = jiji()
        .current_dir(repo)
        .arg("status")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .wrap_err("failed to spawn jiji status")?;

    assert_child_stays_running(&mut child, Duration::from_millis(150))
        .wrap_err("status should wait while another process holds the repository lock")?;

    lock_file
        .unlock()
        .wrap_err("failed to release exclusive repository lock")?;
    drop(lock_file);

    let output = wait_with_timeout(child, Duration::from_secs(5))
        .wrap_err("failed to wait for jiji status")?;
    assert!(output.status.success(), "jiji status should succeed");

    let stdout = String::from_utf8(output.stdout).wrap_err("stdout should be UTF-8")?;
    let stderr = String::from_utf8(output.stderr).wrap_err("stderr should be UTF-8")?;

    assert!(
        stdout.contains("Status:"),
        "status output should remain on stdout: {stdout}"
    );
    assert!(
        !stdout.contains("Waiting for repository lock"),
        "wait message must not pollute stdout: {stdout}"
    );
    assert!(
        stderr.contains("Waiting for repository lock"),
        "wait message should be written to stderr: {stderr}"
    );

    Ok(())
}
