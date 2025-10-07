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
    let current_directory = current_dir().wrap_err("failed to get current directory")?;
    let current_directory = Utf8PathBuf::try_from(current_directory)
        .wrap_err("current directory is not valid utf-8")?;
    JijiRepository::find_upwards_from(current_directory).wrap_err("failed to find repository root")
}
