# Jiji

Jiji is a lightweight, Git-inspired file storage system designed for managing large machine learning datasets, models, and other assets.
Jiji helps developers track changes, store files efficiently, and push or fetch assets to remote storage backends.

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

Push assets to remote storage:

```bash
jiji push sftp_storage
```

Fetch assets from remote storage:

```bash
jiji fetch sftp_storage
```

## License

This project is licensed under the [Apache License 2.0](LICENSE).

## Contributions

Contributions are welcome! If you encounter issues, or have ideas for new storage backends, improved indexing, or performance enhancements, please open a pull request or issue.
