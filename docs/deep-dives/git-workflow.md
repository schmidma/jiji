# Git Workflow With Jiji

Jiji is designed to fit into a normal Git repository without taking over Git.

Jiji creates and updates metadata that Git can track, but it never stages files, creates commits, or rewrites Git history. Review, stage, and commit Jiji-related metadata with normal Git commands.

## The Boundary

Jiji owns large-file metadata and cache operations. Git owns source history.

Jiji may create or update these Git-visible files:

- `*.jiji` reference files
- `.jiji/config.toml`
- `.jiji/.gitignore`
- local `.gitignore` files containing Jiji-managed blocks

Jiji also creates local runtime state that should stay out of Git:

- `.jiji/cache/`
- `.jiji/.lock`
- `.jiji/config.local.toml` when local overrides are added in the future

Jiji-managed `.gitignore` rules hide untracked materialized files and directories after `jiji add`, so ordinary `git status` output stays focused on metadata that should be reviewed and committed.

## What To Commit

In normal team workflows, commit:

- `*.jiji` files, because they describe tracked content
- `.jiji/config.toml`, because it stores shared repository storage configuration
- `.jiji/.gitignore`, because it keeps Jiji local runtime state out of Git
- Jiji-managed `.gitignore` changes, because they keep materialized tracked content out of normal Git workflows

Do not commit:

- `.jiji/cache/`
- `.jiji/.lock`
- materialized files or directories that Jiji has added to a managed `.gitignore` block

## Existing Git-Tracked Files

Jiji-managed `.gitignore` rules only hide untracked materialized content. If Git already tracks a file or directory before `jiji add`, Git will continue tracking it until you explicitly remove it from the Git index.

Jiji does not run `git rm --cached` for you. To migrate an already Git-tracked file to Jiji tracking, review the result and remove the materialized file from Git history going forward with normal Git commands:

```bash
git rm --cached model.bin
git add model.bin.jiji .gitignore
git commit -m "data: track model artifact with jiji"
```

The working-tree file remains on disk; only Git's index stops tracking it.

## After `jiji init`

`jiji init` creates shared configuration and local repository state.

Expected Git-visible changes:

- `.jiji/config.toml`
- `.jiji/.gitignore`

Expected local-only state:

- `.jiji/cache/`
- `.jiji/.lock`

Typical Git follow-up:

```bash
git add .jiji/config.toml .jiji/.gitignore
git commit -m "chore: initialize jiji"
```

## After `jiji add model.bin`

`jiji add model.bin` stores the file content in Jiji's cache, writes a Git-visible reference file, and updates a local managed ignore block.

Expected Git-visible changes:

- `model.bin.jiji`
- `.gitignore` containing a Jiji-managed rule for `/model.bin`

Expected ignored materialized content:

- `model.bin`

Typical Git follow-up:

```bash
git add model.bin.jiji .gitignore
git commit -m "data: track model artifact with jiji"
```

## After `jiji add data/images`

Tracked directories use the same pattern, with metadata and ignore rules placed next to the tracked directory root.

Expected Git-visible changes:

- `data/images.jiji`
- `data/.gitignore` containing a Jiji-managed rule for `/images/`

Expected ignored materialized content:

- `data/images/`

Typical Git follow-up:

```bash
git add data/images.jiji data/.gitignore
git commit -m "data: track image directory with jiji"
```

## After `jiji untrack model.bin`

`jiji untrack` removes Jiji metadata and updates managed ignore rules, but it leaves working-tree files on disk.

Expected Git-visible changes:

- removal of `model.bin.jiji`
- update or removal of the Jiji-managed block in `.gitignore`

Expected working-tree content:

- `model.bin` remains on disk
- `model.bin` may become visible to Git again if no other ignore rule covers it

Typical Git follow-up:

```bash
git add -u model.bin.jiji .gitignore
git status
```

If you want Git to track `model.bin` as a normal file after untracking it from Jiji, add and commit it explicitly:

```bash
git add model.bin
git commit -m "data: stop tracking model artifact with jiji"
```

If you want to keep `model.bin` local-only, add or preserve your own ignore rule outside the Jiji-managed block.

## What Jiji Does Not Do

Jiji does not run these commands for you:

```bash
git add
git commit
git reset
git checkout
```

Jiji may make Git-visible metadata changes, but reviewing and committing those changes stays explicit.
