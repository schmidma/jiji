# Jiji

Jiji is a lightweight tool for tracking and storing large files inside a Git repository.
It keeps file contents in a content-addressed cache, records tracked state in `*.jiji` reference files, and can push or fetch cached objects through configured storage backends.

> [!WARNING]
> This project is currently in **alpha**. It is **far from finished**, and APIs, features, and behavior may change at any time. Use at your own risk — things may break, and documentation may be incomplete.
>
> Contributions, feedback, and bug reports are welcome, but please be aware that stability is not guaranteed.

## Key Features

- **Content-addressable storage** using hashes to uniquely identify files
- **Multiple storage backends**: local filesystem, SFTP, with more planned
- **Efficient directory handling** with automatic indexing of files and directories
- **Track untracked, modified, and deleted files** to maintain reproducible datasets
- **Portable and lightweight**, tailored for machine learning projects

## Installation

### From GitHub

```bash
cargo install --git https://github.com/schmidma/jiji.git
```

### From Release

Download the latest release from [Releases](https://github.com/schmidma/jiji/releases).

> Currently, Jiji is not published on crates.io. (yet)

## Basic Usage

Initialize a new repository:

```bash
jiji init
```

Add files or directories to tracking:

```bash
jiji add data/models/resnet50.pt
jiji add data/datasets
```

Check repository status:

```bash
jiji status
# Output example:
#     modified: data/models/resnet50.pt
#     untracked: data/datasets/new_dataset.csv
```

Configure storage backends:

```bash
jiji storage add local file:///srv/jiji-cache
jiji storage add backup sftp://alice@example.com:/srv/jiji
```

List configured storages:

```bash
jiji storage list
jiji storage list --detail
```

The first storage you add becomes the default automatically. A repository may also have configured storages with no default at all. Commands that require a default storage, such as `push` and `fetch`, will error with guidance to run `jiji storage default <name>` or inspect `jiji storage list` if none is configured.

Push assets to the configured default storage:

```bash
jiji push
```

Fetch assets from the configured default storage into `.jiji/cache`:

```bash
jiji fetch
```

`jiji fetch` downloads cached objects, but it does not restore files into the working tree by itself. Use `jiji restore` after `fetch` when you want to materialize tracked content back into the repository tree.

## Repository Locking

Jiji coordinates its currently lock-integrated commands with a repository-wide lock file at `.jiji/.lock`.

- Read-only integrated commands can hold shared locks at the same time.
- Integrated commands that mutate repository state or the working tree wait for exclusive access.
- `jiji init` bootstraps `.jiji/.lock` first, then takes the repository write lock while completing initialization.
- If an integrated command has to block, Jiji writes a waiting message to stderr before sleeping on the lock.
- Locking in the current v1 implementation is blocking-only: there is no timeout and no immediate-fail mode.

Clean unreachable cache objects conservatively:

```bash
jiji gc --dry-run
jiji gc
```

Today, `push` and `fetch` both use the configured default storage. `fetch` downloads objects into `.jiji/cache` and still requires `jiji restore` to write tracked content back into the working tree. `gc --dry-run` reports unreachable cached objects without deleting them, and `gc` removes cached objects that are no longer referenced by tracked files or tracked-directory manifests.

For focused documentation on Jiji's current behavior and internal model, start with [`docs/index.md`](docs/index.md). The locking deep dive is at [`docs/deep-dives/locking.md`](docs/deep-dives/locking.md).

## License

This project is licensed under the [Apache License 2.0](LICENSE).

## Contributions

Contributions are welcome. If you encounter issues, or have ideas for new storage backends, improved indexing, or performance enhancements, please open a pull request or issue.

Before doing development work, read [`docs/workflow.md`](docs/workflow.md) for the repository workflow. Jiji keeps `main` linear and expects unpublished branches to be rewritten into one or a very small stack of semantic atomic commits before integration.
