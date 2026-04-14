# Storage In Jiji

Jiji stores storage backend configuration separately from cached content:

- `.jiji/config.toml` stores configured backends and the optional default storage name
- `.jiji/cache/<2-hex>/<rest>` stores content-addressed cache objects and tracked-directory manifests

That split matters: storage configuration decides where `push` and `fetch` talk to, while the cache is the local source of truth that those commands upload from and download into. `fetch` populates `.jiji/cache`; it does not restore files into the repository tree by itself.

## Storage Model

A repository may have multiple configured storages.

For example, one repository can keep a local filesystem backup and an SFTP remote at the same time:

```bash
jiji storage add local file:///srv/jiji-cache
jiji storage add backup sftp://alice@example.com:/srv/jiji
```

The first storage added becomes the default automatically. After that, the default can be changed with `jiji storage default <name>`.

Having no default storage is still a valid repository state. That can happen if no storage has been added yet, or if the current default was removed. In that state, commands that need a default storage currently error with guidance to run `jiji storage default <name>` or inspect `jiji storage list`.

## Supported URIs Today

Jiji currently accepts these configured storage URI schemes:

- `file://...`
- `sftp://...`

`file://` stores objects on a local filesystem path.

`sftp://` stores objects on an SFTP target. The current parser supports usernames, optional passwords, optional ports, and a required remote location.

Any other URI scheme is rejected during `jiji storage add`.

## Listing Storages

`jiji storage list` prints the configured default name first, then the configured storage entries sorted by storage name.

Example:

```text
Default storage: backup
backup [sftp] (default)
local [file]
```

If no storages are configured, the command reports both facts explicitly:

```text
Default storage: none
No storages configured.
```

`jiji storage list --detail` adds the reconstructed URI and backend-specific fields for each storage.

Example:

```text
Default storage: backup
backup [sftp] (default)
  uri: sftp://alice:secret@example.com:2222:/srv/jiji
  username: alice
  host: example.com
  password: secret
  port: 2222
  location: /srv/jiji
local [file]
  uri: file:///srv/jiji-cache
  location: /srv/jiji-cache
```

`--detail` shows exactly what is configured, including secrets such as SFTP passwords. It is meant for inspection, not redacted display.

## Default Storage Behavior

`push` and `fetch` currently always use the configured default storage.

`fetch` downloads objects into `.jiji/cache`. It does not materialize files into the working tree by itself; use `jiji restore` after `fetch` when you want tracked content written back into the repository tree.

There is no per-command storage override yet. If a command requires a default storage and none is configured, Jiji errors instead of guessing:

- set one with `jiji storage default <name>`
- inspect the configured choices with `jiji storage list`

Jiji also validates that the configured default name still exists in `.jiji/config.toml` before using it.

## First-Version Garbage Collection

`jiji gc` is intentionally conservative in its first version.

It walks the repository index, collects every cache object that is still reachable from tracked files and tracked-directory manifests, then scans `.jiji/cache` for on-disk objects that are no longer referenced.

`jiji gc --dry-run` reports what would be removed without deleting anything.

`jiji gc` removes unreferenced cache objects and reports how many objects it kept and removed.

Current conservative limits:

- GC only acts on local cache objects under `.jiji/cache`
- GC keeps every cache object reachable from the current tracked state
- GC errors if a tracked file object is missing from the cache instead of continuing with partial information
- GC errors if a tracked-directory manifest is missing from the cache instead of trying to rebuild or skip it
- GC does not reach into configured remote storages; it is local cache cleanup only

That behavior favors safety over reclaiming space aggressively. If the repository metadata says an object should exist locally, GC requires that object to be present before it decides what is safe to remove.

## Code References

- `src/configuration.rs`: storage URI parsing, persisted configuration, default-storage enforcement, and storage listing data
- `src/bin/jiji.rs`: CLI commands for `storage list`, `storage list --detail`, `storage add`, and `gc`
- `src/cache.rs`: local cache layout under `.jiji/cache/<2-hex>/<rest>`
- `src/gc.rs`: local cache reachability scan and removal behavior
