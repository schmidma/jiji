use std::env::current_dir;

use camino::{absolute_utf8, Utf8PathBuf};
use clap::{Parser, Subcommand};
use color_eyre::{
    eyre::{bail, Context as _, ContextCompat as _},
    Result,
};
use jiji::JijiRepository;
use tracing::Level;

#[derive(Debug, Parser)]
struct Arguments {
    #[clap(subcommand)]
    command: Command,
    /// Enable verbose output
    #[clap(short, long)]
    verbose: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize a repository
    Init {
        /// Path to the repository root
        #[clap(default_value = ".")]
        path: Utf8PathBuf,
    },
    /// Add a file
    Add {
        /// Path to the file to add
        paths: Vec<Utf8PathBuf>,
        /// Path to the repository root
        #[clap(short, long)]
        repository_root: Option<Utf8PathBuf>,
    },
    /// Restore a file
    Restore {
        /// Path to the file to restore
        paths: Vec<Utf8PathBuf>,
        /// Path to the repository root
        #[clap(short, long)]
        repository_root: Option<Utf8PathBuf>,
    },
    /// Show the status of the repository
    Status {
        /// Path to the repository root
        #[clap(short, long)]
        repository_root: Option<Utf8PathBuf>,
    },
    /// Run automatic garbage collection
    GC {
        /// Path to the repository root
        #[clap(short, long)]
        repository_root: Option<Utf8PathBuf>,
    },
    /// Manage storage backends
    Storage {
        /// Path to the repository root
        #[clap(short, long)]
        repository_root: Option<Utf8PathBuf>,
        #[clap(subcommand)]
        command: StorageCommand,
    },
    /// Push local files to the remote storage backend
    Push {
        /// Path to the repository root
        #[clap(short, long)]
        repository_root: Option<Utf8PathBuf>,
    },
    /// Fetch files from the remote storage backend
    Fetch {
        /// Path to the repository root
        #[clap(short, long)]
        repository_root: Option<Utf8PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum StorageCommand {
    /// List all configured storage backends
    List,
    /// Add a new storage backend
    Add {
        /// Name of the storage backend
        name: String,
        /// URI of the storage backend
        uri: String,
    },
    /// Remove a storage backend
    Remove {
        /// Name of the storage backend to remove
        name: String,
    },
    /// Set the default storage backend
    Default {
        /// Name of the storage backend
        name: String,
    },
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let arguments = Arguments::parse();
    let max_level = if arguments.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };
    tracing_subscriber::fmt()
        .pretty()
        .without_time()
        .with_target(false)
        .with_max_level(max_level)
        .init();

    match arguments.command {
        Command::Init { path } => {
            JijiRepository::init(path).wrap_err("failed to run init command")?;
        }
        Command::Add {
            paths,
            repository_root,
        } => {
            let repository = resolve_repository_root(repository_root)?;
            repository
                .add(&paths)
                .wrap_err("failed to run add command")?;
        }
        Command::Restore {
            paths,
            repository_root,
        } => {
            let repository = resolve_repository_root(repository_root)?;
            repository
                .restore(&paths)
                .wrap_err("failed to run restore command")?;
        }
        Command::Status { repository_root } => {
            let repository = resolve_repository_root(repository_root)?;
            repository
                .status()
                .wrap_err("failed to run status command")?;
        }
        Command::GC { .. } => todo!(),
        Command::Storage {
            repository_root,
            command,
        } => {
            let mut repository = resolve_repository_root(repository_root)?;
            match command {
                StorageCommand::List => {
                    todo!()
                }
                StorageCommand::Add { name, uri } => {
                    repository
                        .add_storage(&name, &uri)
                        .wrap_err("failed to add storage")?;
                }
                StorageCommand::Remove { name } => {
                    repository
                        .remove_storage(&name)
                        .wrap_err("failed to remove storage")?;
                }
                StorageCommand::Default { name } => {
                    repository
                        .set_default_storage(&name)
                        .wrap_err("failed to set default storage")?;
                }
            }
        }
        Command::Push { repository_root } => {
            let repository = resolve_repository_root(repository_root)?;
            let storage = repository.config().default_storage.as_ref().wrap_err("no default storage configured. Please specify a storage backend in the repository configuration.")?;
            repository
                .push(storage)
                .wrap_err("failed to run push command")?;
        }
        Command::Fetch { repository_root } => {
            let repository = resolve_repository_root(repository_root)?;
            let storage = repository.config().default_storage.as_ref().wrap_err("no default storage configured. Please specify a storage backend in the repository configuration.")?;
            repository
                .fetch(storage)
                .wrap_err("failed to run fetch command")?;
        }
    }
    Ok(())
}

fn resolve_repository_root(repository_root: Option<Utf8PathBuf>) -> Result<JijiRepository> {
    let current_directory = utf8_current_dir()?;
    resolve_repository_root_from_current_directory(repository_root, current_directory)
}

fn resolve_repository_root_from_current_directory(
    repository_root: Option<Utf8PathBuf>,
    current_directory: Utf8PathBuf,
) -> Result<JijiRepository> {
    if let Some(root) = repository_root {
        let root = absolute_utf8(root).wrap_err("failed to get absolute path")?;
        let repository = JijiRepository::new(root).wrap_err("failed to open repository")?;
        if !repository.is_initialized() {
            bail!(
                "no jiji repository found at '{}'",
                repository.workspace_root(),
            )
        }
        return Ok(repository);
    }
    JijiRepository::find_upwards_from(current_directory).wrap_err("failed to find repository root")
}

fn utf8_current_dir() -> Result<Utf8PathBuf> {
    let current_directory = current_dir().wrap_err("failed to get current directory")?;
    Utf8PathBuf::try_from(current_directory).wrap_err("current directory is not valid utf-8")
}

#[cfg(test)]
mod tests {
    use std::{
        env::{current_dir, set_current_dir},
        fs::create_dir_all,
        sync::{Mutex, MutexGuard, OnceLock},
    };

    use camino::Utf8Path;
    use color_eyre::eyre::Context as _;
    use tempfile::tempdir;

    use super::*;

    struct CurrentDirGuard {
        original: Utf8PathBuf,
        _lock: MutexGuard<'static, ()>,
    }

    impl CurrentDirGuard {
        fn set(path: &Utf8Path) -> Result<Self> {
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

    fn resolve_input_paths_from(
        repository: &JijiRepository,
        working_directory: &Utf8Path,
        paths: &[Utf8PathBuf],
    ) -> Result<Vec<Utf8PathBuf>> {
        paths
            .iter()
            .map(|path| repository.to_repo_relative_path_from(path, working_directory))
            .collect()
    }

    #[test]
    fn resolve_command_paths_uses_explicit_working_directory() -> Result<()> {
        let repo_dir = tempdir()?;
        let repo_root = <&Utf8Path>::try_from(repo_dir.path())?;
        let repository = JijiRepository::init(repo_root)?;
        let working_directory = repo_root.join("nested/deeper");
        create_dir_all(working_directory.as_std_path())?;

        let resolved =
            resolve_input_paths_from(&repository, &working_directory, &["file.txt".into()])?;

        assert_eq!(resolved, vec![Utf8PathBuf::from("nested/deeper/file.txt")]);

        Ok(())
    }

    #[test]
    fn resolve_command_paths_treats_nested_input_as_cwd_relative_from_nested_cwd() -> Result<()> {
        let repo_dir = tempdir()?;
        let repo_root = <&Utf8Path>::try_from(repo_dir.path())?;
        let repository = JijiRepository::init(repo_root)?;
        let working_directory = repo_root.join("nested/deeper");
        create_dir_all(working_directory.as_std_path())?;

        let resolved = resolve_input_paths_from(
            &repository,
            &working_directory,
            &["nested/deeper/file.txt".into()],
        )?;

        assert_eq!(
            resolved,
            vec![Utf8PathBuf::from("nested/deeper/nested/deeper/file.txt")]
        );

        Ok(())
    }

    #[test]
    fn add_command_paths_work_for_bare_and_nested_inputs() -> Result<()> {
        let repo_dir = tempdir()?;
        let repo_root = <&Utf8Path>::try_from(repo_dir.path())?;
        let repository = JijiRepository::init(repo_root)?;
        let working_directory = repo_root.join("nested/deeper");
        create_dir_all(repo_root.join("nested/deeper/nested/deeper").as_std_path())?;
        let _guard = CurrentDirGuard::set(&working_directory)?;

        std::fs::write(repo_root.join("nested/deeper/bare.txt"), "bare")?;
        std::fs::write(
            repo_root.join("nested/deeper/nested/deeper/repo.txt"),
            "repo",
        )?;

        repository.add(["bare.txt", "nested/deeper/repo.txt"])?;

        assert!(repo_root.join("nested/deeper/bare.txt.jiji").exists());
        assert!(repo_root
            .join("nested/deeper/nested/deeper/repo.txt.jiji")
            .exists());

        Ok(())
    }

    #[test]
    fn restore_command_paths_work_for_bare_and_nested_inputs() -> Result<()> {
        let repo_dir = tempdir()?;
        let repo_root = <&Utf8Path>::try_from(repo_dir.path())?;
        let repository = JijiRepository::init(repo_root)?;
        let working_directory = repo_root.join("nested/deeper");
        create_dir_all(repo_root.join("nested/deeper/nested/deeper").as_std_path())?;
        let _guard = CurrentDirGuard::set(&working_directory)?;

        std::fs::write(repo_root.join("nested/deeper/bare.txt"), "bare")?;
        std::fs::write(
            repo_root.join("nested/deeper/nested/deeper/repo.txt"),
            "repo",
        )?;
        let bare_path = repo_root.join("nested/deeper/bare.txt");
        let nested_path = repo_root.join("nested/deeper/nested/deeper/repo.txt");
        repository.add([bare_path.as_str(), nested_path.as_str()])?;

        std::fs::write(repo_root.join("nested/deeper/bare.txt"), "modified bare")?;
        std::fs::write(
            repo_root.join("nested/deeper/nested/deeper/repo.txt"),
            "modified repo",
        )?;

        repository.restore(&["bare.txt", "nested/deeper/repo.txt"])?;

        assert_eq!(
            std::fs::read_to_string(repo_root.join("nested/deeper/bare.txt"))?,
            "bare"
        );
        assert_eq!(
            std::fs::read_to_string(repo_root.join("nested/deeper/nested/deeper/repo.txt"))?,
            "repo"
        );

        Ok(())
    }

    #[test]
    fn resolve_repository_root_uses_utf8_current_directory() -> Result<()> {
        let repo_dir = tempdir()?;
        let repo_root = <&Utf8Path>::try_from(repo_dir.path())?;
        let nested = repo_root.join("nested/deeper");

        JijiRepository::init(repo_root)?;
        create_dir_all(nested.as_std_path())?;

        let repository = resolve_repository_root_from_current_directory(None, nested)?;

        assert_eq!(
            repository.workspace_root(),
            Utf8PathBuf::from("../../.jiji")
        );

        Ok(())
    }
}
