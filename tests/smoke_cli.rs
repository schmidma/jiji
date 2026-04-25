use std::{
    ffi::OsStr,
    fs,
    process::{Command, Output},
};

use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{eyre::bail, eyre::Context as _, Result};
use tempfile::tempdir;

fn jiji() -> Command {
    Command::new(env!("CARGO_BIN_EXE_jiji"))
}

fn run(repo: &Utf8Path, args: &[&str]) -> Result<Output> {
    let output = jiji()
        .current_dir(repo)
        .args(args)
        .output()
        .wrap_err_with(|| format!("failed to run jiji {}", args.join(" ")))?;

    if !output.status.success() {
        bail!(
            "jiji {} failed with status {}\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(output)
}

fn run_os(repo: &Utf8Path, args: &[&OsStr]) -> Result<Output> {
    let command = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let output = jiji()
        .current_dir(repo)
        .args(args)
        .output()
        .wrap_err_with(|| format!("failed to run jiji {command}"))?;

    if !output.status.success() {
        bail!(
            "jiji {command} failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(output)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn assert_contains(haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "expected to find {needle:?} in:\n{haystack}"
    );
}

fn assert_not_contains(haystack: &str, needle: &str) {
    assert!(
        !haystack.contains(needle),
        "did not expect to find {needle:?} in:\n{haystack}"
    );
}

#[test]
fn cli_smoke_repository_storage_and_cleanup_workflow() -> Result<()> {
    color_eyre::install().ok();

    let repo_dir = tempdir()?;
    let repo = Utf8PathBuf::from_path_buf(repo_dir.path().to_path_buf())
        .map_err(|path| color_eyre::eyre::eyre!("temp path is not UTF-8: {}", path.display()))?;
    let storage_dir = tempdir()?;
    let storage = Utf8PathBuf::from_path_buf(storage_dir.path().to_path_buf())
        .map_err(|path| color_eyre::eyre::eyre!("temp path is not UTF-8: {}", path.display()))?;
    let storage_uri = format!("file://{storage}");

    run_os(
        Utf8Path::new("."),
        &[OsStr::new("init"), repo.as_std_path().as_os_str()],
    )?;

    assert!(repo.join(".jiji").is_dir());
    assert!(repo.join(".jiji/cache").is_dir());
    assert!(repo.join(".jiji/config.toml").is_file());
    let workspace_gitignore = fs::read_to_string(repo.join(".jiji/.gitignore"))?;
    assert_contains(&workspace_gitignore, "/cache/");
    assert_contains(&workspace_gitignore, "/.lock");
    assert_contains(&workspace_gitignore, "/config.local.toml");

    fs::create_dir_all(repo.join("data/images"))?;
    fs::write(repo.join("model.bin"), "original model")?;
    fs::write(repo.join("data/images/photo.jpg"), "original photo")?;
    fs::write(repo.join("data/images/labels.txt"), "original labels")?;

    run(&repo, &["add", "model.bin", "data/images"])?;

    assert!(repo.join("model.bin.jiji").is_file());
    assert!(repo.join("data/images.jiji").is_file());

    let root_gitignore = fs::read_to_string(repo.join(".gitignore"))?;
    assert_eq!(
        root_gitignore,
        "# BEGIN Jiji tracked content\n/model.bin\n# END Jiji tracked content\n"
    );
    assert_not_contains(&root_gitignore, "*.jiji");

    let data_gitignore = fs::read_to_string(repo.join("data/.gitignore"))?;
    assert_eq!(
        data_gitignore,
        "# BEGIN Jiji tracked content\n/images/\n# END Jiji tracked content\n"
    );
    assert_not_contains(&data_gitignore, "*.jiji");

    let status = stdout(&run(&repo, &["status"])?);
    assert_contains(&status, "Status: No changes, clean");

    fs::write(repo.join("model.bin"), "modified model")?;
    fs::remove_file(repo.join("data/images/photo.jpg"))?;

    let dirty_status = stdout(&run(&repo, &["status"])?);
    assert_contains(&dirty_status, "modified");
    assert_contains(&dirty_status, "model.bin");
    assert_contains(&dirty_status, "deleted");
    assert_contains(&dirty_status, "data/images/photo.jpg");

    run(&repo, &["restore", "model.bin", "data/images"])?;

    assert_eq!(
        fs::read_to_string(repo.join("model.bin"))?,
        "original model"
    );
    assert_eq!(
        fs::read_to_string(repo.join("data/images/photo.jpg"))?,
        "original photo"
    );
    assert_eq!(
        fs::read_to_string(repo.join("data/images/labels.txt"))?,
        "original labels"
    );

    let restored_status = stdout(&run(&repo, &["status"])?);
    assert_contains(&restored_status, "Status: No changes, clean");

    run(&repo, &["storage", "add", "local", &storage_uri])?;

    let storage_list = stdout(&run(&repo, &["storage", "list", "--detail"])?);
    assert_contains(&storage_list, "Default storage: local");
    assert_contains(&storage_list, "local [file] (default)");
    assert_contains(&storage_list, &storage_uri);

    run(&repo, &["storage", "default", "local"])?;
    run(&repo, &["push"])?;

    fs::remove_dir_all(repo.join(".jiji/cache"))?;
    fs::create_dir_all(repo.join(".jiji/cache"))?;

    run(&repo, &["fetch"])?;

    fs::remove_file(repo.join("model.bin"))?;
    fs::remove_dir_all(repo.join("data/images"))?;
    run(&repo, &["restore", "model.bin", "data/images"])?;

    assert_eq!(
        fs::read_to_string(repo.join("model.bin"))?,
        "original model"
    );
    assert_eq!(
        fs::read_to_string(repo.join("data/images/photo.jpg"))?,
        "original photo"
    );
    assert_eq!(
        fs::read_to_string(repo.join("data/images/labels.txt"))?,
        "original labels"
    );

    let gc_dry_run = stdout(&run(&repo, &["gc", "--dry-run"])?);
    assert_contains(
        &gc_dry_run,
        "GC dry run complete: kept 4 objects, would remove 0 objects",
    );

    let gc = stdout(&run(&repo, &["gc"])?);
    assert_contains(&gc, "GC complete: kept 4 objects, removed 0 objects");

    run(&repo, &["storage", "remove", "local"])?;
    let storage_list_after_remove = stdout(&run(&repo, &["storage", "list"])?);
    assert_contains(&storage_list_after_remove, "Default storage: none");
    assert_contains(&storage_list_after_remove, "No storages configured.");

    run(&repo, &["untrack", "model.bin", "data/images"])?;

    assert!(!repo.join("model.bin.jiji").exists());
    assert!(!repo.join("data/images.jiji").exists());
    assert!(repo.join("model.bin").is_file());
    assert!(repo.join("data/images/photo.jpg").is_file());
    assert!(repo.join("data/images/labels.txt").is_file());
    assert!(
        !repo.join(".gitignore").exists(),
        "root .gitignore should be removed after its only Jiji-managed rule is removed"
    );
    assert!(
        !repo.join("data/.gitignore").exists(),
        "data/.gitignore should be removed after its only Jiji-managed rule is removed"
    );

    Ok(())
}
