# Repository Locking

Jiji currently coordinates repository access through a single lock file at `.jiji/.lock`.

## Scope

- The lock is repository-wide within a Jiji worktree: commands that open the same repository root all contend on the same `.jiji/.lock` file.
- The lock file lives under `.jiji/`, alongside the rest of Jiji's local repository metadata.
- Locking is local to the filesystem semantics behind that file. Jiji does not add extra distributed coordination on top.

## Lock Modes

Jiji uses a simple reader/writer model.

- Read locks allow overlapping readers.
- Write locks require exclusive access.
- There is no lock upgrade path. A command does not acquire a read lock first and then promote it to a write lock.

## Current Command Classes

Read-locked commands:

- `jiji status`
- `jiji storage list`

Write-locked commands:

- `jiji init` after `.jiji/.lock` has been bootstrapped
- `jiji add`
- `jiji restore`
- `jiji untrack`
- `jiji gc`
- `jiji storage add`
- `jiji storage remove`
- `jiji storage default`
- `jiji push`
- `jiji fetch`

These classes reflect the current v1 implementation for commands that already participate in repository locking. Integrated commands that mutate repository metadata, cache state, storage configuration, or the working tree take the write lock. Read-only inspection commands take the read lock.

`jiji init` is the bootstrap case: it creates `.jiji/` and `.jiji/.lock` if needed, then takes the write lock before creating or repairing the remaining required repository files.

## Blocking Behavior

Jiji first tries to acquire the requested lock mode without blocking.

- If the lock is available, the command continues immediately.
- If the lock would block, Jiji writes `Waiting for repository lock at '<path>' while running <command>...` to stderr and then waits until the lock becomes available.
- Multiple readers can overlap, but a writer waits until all readers release the lock.
- A reader also waits if another command already holds the write lock.

## Deliberate v1 Limits

- No timeout support.
- No immediate-fail or try-lock CLI mode.
- No lock upgrades.
- Local-filesystem semantics only.

Those limits are intentional in the current implementation: locking is metadata-free, blocking-only, and centered on the local `.jiji/.lock` file.
