use camino::{absolute_utf8, Utf8Path, Utf8PathBuf};
use color_eyre::eyre::{Context as _, ContextCompat as _};
use color_eyre::owo_colors::OwoColorize as _;
use color_eyre::Result;
use std::fmt::{self, Display, Formatter, Write as _};
use tracing::warn;

use crate::index::{DirectoryChildren, FileStatus, Node};
use crate::relative_path::AsRelativePath as _;
use crate::JijiRepository;

#[derive(Debug, Clone)]
pub enum StatusKind {
    Modified,
    Deleted,
    Unknown,
    Untracked,
}

impl Display for StatusKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Modified => write!(f, "modified"),
            Self::Deleted => write!(f, "deleted"),
            Self::Unknown => write!(f, "unknown"),
            Self::Untracked => write!(f, "untracked"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StatusEntry {
    pub path: Utf8PathBuf,
    pub status: StatusKind,
}

#[derive(Debug, Default)]
pub struct StatusReport {
    entries: Vec<StatusEntry>,
}

impl StatusReport {
    fn add_entry(&mut self, path: Utf8PathBuf, status: StatusKind) {
        self.entries.push(StatusEntry { path, status });
    }

    fn add_file_entry(&mut self, path: Utf8PathBuf, status: &FileStatus) {
        match status {
            FileStatus::Modified { .. } => self.add_entry(path, StatusKind::Modified),
            FileStatus::Deleted => self.add_entry(path, StatusKind::Deleted),
            FileStatus::Unknown => {
                warn!("unknown file status for {path}");
                self.add_entry(path, StatusKind::Unknown);
            }
            FileStatus::Clean | FileStatus::Staged => {}
        }
    }

    fn append_node_status(&mut self, node: &Node, repo: &JijiRepository) -> Result<()> {
        for file in &node.files {
            let path = node.base.join(&file.path);
            self.add_file_entry(path, &file.status);
        }

        for directory in &node.directories {
            let dir_path = node.base.join(&directory.path);

            let DirectoryChildren::Resolved(children) = &directory.children else {
                warn!("unresolved directory children for {dir_path}");
                continue;
            };

            for file in children {
                let path = dir_path.join(&file.path);
                self.add_file_entry(path, &file.status);
            }

            for entry in walkdir::WalkDir::new(repo.root.join(&dir_path))
                .sort_by_file_name()
                .into_iter()
                .filter_map(|entry| match entry {
                    Ok(entry) => Some(entry),
                    Err(error) => {
                        warn!("failed to read entry: {error:#}");
                        None
                    }
                })
                .filter(|entry| entry.file_type().is_file())
            {
                let entry_path =
                    Utf8Path::from_path(entry.path()).wrap_err("path is not valid UTF-8")?;
                let child_path = entry_path
                    .strip_prefix(repo.root.join(&dir_path))
                    .expect("entry is child of directory");
                if !children.iter().any(|child| child.path == child_path) {
                    self.add_entry(entry_path.to_owned(), StatusKind::Untracked);
                }
            }
        }

        Ok(())
    }

    const fn is_clean(&self) -> bool {
        self.entries.is_empty()
    }

    fn format(&self) -> Result<String> {
        let mut output = String::new();
        for entry in &self.entries {
            let path = absolute_utf8(&entry.path)?.as_relative_path()?;
            let line = format!("\t{:10}: {}", entry.status, path).red().to_string();
            writeln!(&mut output, "{line}")?;
        }
        Ok(output)
    }
}

impl JijiRepository {
    /// Prints the repository status in a structured way.
    pub fn status(&self) -> Result<()> {
        let mut index = self.index().wrap_err("failed to collect index")?;
        index
            .resolve_status(self)
            .wrap_err("failed to resolve status")?;

        let mut report = StatusReport::default();
        for node in index.iter_nodes() {
            report.append_node_status(node, self).wrap_err_with(|| {
                format!(
                    "failed to collect status for reference file '{}'",
                    node.path
                )
            })?;
        }

        if report.is_clean() {
            println!("Status: No changes, clean");
        } else {
            let status = report.format()?;
            println!("Status:\n{status}");
        }

        Ok(())
    }
}
