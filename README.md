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

Push assets to the configured default storage:

```bash
jiji push
```

Fetch assets from the configured default storage:

```bash
jiji fetch
```

For focused documentation on Jiji's current behavior and internal model, start with [`docs/index.md`](docs/index.md).

## License

This project is licensed under the [Apache License 2.0](LICENSE).

## Contributions

Contributions are welcome. If you encounter issues, or have ideas for new storage backends, improved indexing, or performance enhancements, please open a pull request or issue.

Before doing development work, read [`docs/workflow.md`](docs/workflow.md) for the repository workflow. Jiji keeps `main` linear and expects unpublished branches to be rewritten into one or a very small stack of semantic atomic commits before integration.
