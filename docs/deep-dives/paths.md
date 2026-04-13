# Paths In Jiji

Jiji has a two-layer path model:

1. User input paths: what a person types into `jiji add` or `jiji restore`.
2. Repository-relative semantic paths: the normalized paths Jiji stores and looks up internally.

Most path behavior gets simpler once you keep those two layers separate.

## Why Paths Matter

Jiji has to accept paths from different working directories, reject paths outside the repository, and still store one stable internal path for each tracked entry.

That is why the CLI first resolves user input relative to the current shell location, then rewrites the result relative to `repo.root`. After that, `add`, `restore`, indexing, and lookups all work from the same repository-relative semantic path.

## User-Facing Rules

### Absolute Paths

Absolute paths are allowed if they point inside the repository.

- Example: `/repo/nested/deeper/file.txt` becomes `nested/deeper/file.txt`
- Outside the repository, the command errors instead of silently tracking the wrong file

`to_repo_relative_path` in `src/lib.rs` is the core helper for this normalization.

### Relative Paths

Every relative input path is resolved from the current working directory first, then rewritten as a repository-relative semantic path.

- From the repository root, `file.txt` stays `file.txt`
- From `repo_root/nested/deeper`, `file.txt` becomes `nested/deeper/file.txt`

Jiji does not preserve repo-relative-looking relative inputs as a second meaning of user input. If a person types a relative path, Jiji always interprets that text relative to the shell's current working directory.

For CLI commands, `src/bin/jiji.rs` does this through `resolve_command_paths(...)`, which uses `to_repo_relative_path_from(...)`.

### Nested Working Directories

Running commands below the repository root does not change that rule: relative input still means cwd-relative input.

If you are in `repo_root/nested/deeper` and type `jiji add nested/deeper/file.txt`, Jiji resolves that to `repo_root/nested/deeper/nested/deeper/file.txt`. If that file does not exist, the command errors.

Repository-relative semantic paths still exist, but only as Jiji's internal normalized paths after input resolution. They are not an alternate interpretation of relative user input.

### Outside-Repository Paths

Paths outside the repository are rejected.

- Example: from `repo_root/nested/deeper`, `jiji add ../../../../tmp/file.txt` should fail once the normalized absolute path no longer falls under `repo.root`

Jiji does not have a mode where tracked paths can point outside the repository tree.

## Internal Model

Those user-facing rules feed a simpler internal model: semantic paths are repository-relative.

That means:

- user input can vary by shell location
- relative user input has one meaning only: cwd-relative
- semantic lookup paths do not vary by shell location
- repo-relative semantic paths are internal normalized paths, not preserved spellings from the CLI
- I/O is anchored back at `repo.root` when Jiji reads, writes, caches, or restores content

`src/add.rs`, `src/restore.rs`, and `src/cache.rs` all join repository-relative paths onto `repo.root` before touching the filesystem.

Persisted `Reference.path` values use narrower bases:

- semantic lookup paths stay repository-relative
- file entries in a node `*.jiji` file are stored relative to the node base
- directory entries in a node `*.jiji` file are stored relative to the node base
- child entries in a tracked-directory manifest are stored relative to the tracked directory root

## Where Paths Are Stored

### Reference Files

Tracked files are described by `*.jiji` TOML reference files stored in the tracked tree.

- A tracked root file like `file.txt` gets `file.txt.jiji`
- A tracked nested file like `nested/deeper/file.txt` gets `nested/deeper/file.txt.jiji`

`src/index.rs` walks the repository for `*.jiji` files and reads them back into the in-memory index.

Those reference files do not persist every `Reference.path` as repository-relative. The node location and base are repository-relative, but contained file and directory `Reference.path` values are base-relative.

### Tracked Directory Children

Tracked directories store a directory reference in a `*.jiji` file, and that reference points to a cached manifest of children.

The node path and base stay repository-relative, the directory entry in the node stays base-relative, and child paths inside the cached manifest stay relative to the tracked directory root.

### Cache Paths

File contents and cached directory manifests live under `.jiji/cache`.

`src/cache.rs` lays them out as `.jiji/cache/<2-hex>/<rest>`, so the cache path is derived from content hash, not from the original filename.

## Worked Examples

Assume the repository root is `/repo`.

### `jiji add file.txt` from repository root

- Shell location: `/repo`
- User input path: `file.txt`
- Internal tracked path: `file.txt`
- On-disk tracked metadata: `file.txt.jiji`
- File I/O anchor: `repo.root.join("file.txt")`

### `jiji add file.txt` from `repo_root/nested/deeper`

- Shell location: `/repo/nested/deeper`
- User input path: `file.txt`
- Internal tracked path: `nested/deeper/file.txt`
- On-disk tracked metadata: `nested/deeper/file.txt.jiji`

This is the main reason `to_repo_relative_path_from(...)` exists.

### `jiji add foo/bar/file.txt` from `repo_root/nested/deeper`

- Shell location: `/repo/nested/deeper`
- User input path: `foo/bar/file.txt`
- Resolved filesystem path: `/repo/nested/deeper/foo/bar/file.txt`
- Internal tracked path: `nested/deeper/foo/bar/file.txt`

### `jiji add nested/deeper/file.txt` from `repo_root/nested/deeper`

- Shell location: `/repo/nested/deeper`
- User input path: `nested/deeper/file.txt`
- Resolved filesystem path: `/repo/nested/deeper/nested/deeper/file.txt`
- Internal tracked path: `nested/deeper/nested/deeper/file.txt`
- Result: error if `/repo/nested/deeper/nested/deeper/file.txt` does not exist

This is the key distinction between external input and internal processing: the user's relative text is cwd-relative first, and only the normalized result becomes a repository-relative semantic path.

### `jiji add ../file.txt` from `repo_root/nested/deeper`

- Shell location: `/repo/nested/deeper`
- User input path: `../file.txt`
- Resolved filesystem path: `/repo/nested/file.txt`
- Internal tracked path: `nested/file.txt`

### `jiji restore file.txt` from `repo_root/nested/deeper`

- Shell location: `/repo/nested/deeper`
- User input path: `file.txt`
- Internal tracked path lookup: `nested/deeper/file.txt`
- Restore target on disk: `repo.root.join("nested/deeper/file.txt")`

`src/restore.rs` then copies content from the hash-derived cache entry back into the repository tree.

### Outside-repository example

- Shell location: `/repo/nested/deeper`
- User input path: `/tmp/file.txt`
- Result: error, because the normalized absolute path is not under `repo.root`

The same rejection applies to relative inputs that escape the repository after normalization.

## Contributor Notes

- Keep the user/input layer separate from the stored/internal layer when changing path code.
- CLI behavior belongs in `src/bin/jiji.rs`; repository normalization belongs in `src/lib.rs`.
- Filesystem writes should stay anchored at `repo.root` so nested cwd behavior does not leak into storage.
- When changing tracked-directory behavior, double-check both the `*.jiji` parent entry and the cached child manifest in `.jiji/cache`.
- `src/index.rs` is the read path for stored semantics; changes there affect how existing reference files are interpreted.

## Code References

- `src/lib.rs`: `to_repo_relative_path(...)`, `to_repo_relative_path_from(...)`, and absolute-path normalization
- `src/bin/jiji.rs`: CLI repository discovery and `resolve_command_paths(...)`
- `src/add.rs`: add-time normalization and writes anchored at `repo.root`
- `src/restore.rs`: restore-time selection and copies anchored at `repo.root`
- `src/cache.rs`: `.jiji/cache` layout and cache writes
- `src/index.rs`: scanning `*.jiji` files and resolving tracked directory children
