# Virtual File System (VFS)

This runtime exposes a sandboxed, in-memory VFS for Node compatibility (`node:fs`, `node:fs/promises`).

## Goals

- Improve ecosystem compatibility for packages that expect `fs` APIs.
- Keep strict sandboxing: no access to host physical filesystem.
- Provide deterministic quota and error behavior.

## Mounts

- `/bundle`
  - Read-only mount reserved for packaged function artifacts.
  - Write operations fail with `EROFS`.
- `/tmp`
  - Writable ephemeral mount in memory.
  - Subject to quota enforcement.
- `/dev/null`
  - Virtual sink.
  - Writes are discarded; reads return EOF/empty content.

## Security Properties

- Host paths are never exposed.
- Only VFS mounts are reachable.
- Escaping via `..` is normalized and prevented from leaving root.
- Writes outside `/tmp` and `/dev/null` fail deterministically.

## Quotas

Two quota dimensions are enforced in writable VFS:

- Total writable quota (`vfsTotalQuotaBytes`): cumulative bytes across files in `/tmp`.
- Per-file quota (`vfsMaxFileBytes`): maximum bytes for a single file.

Default values:

- `vfsTotalQuotaBytes`: `10485760` (10 MiB)
- `vfsMaxFileBytes`: `5242880` (5 MiB)

## Configuration Sources

Configuration precedence (highest to lowest):

1. Function manifest `resources`
2. Runtime global CLI flags (or env vars)
3. Built-in defaults

Manifest fields (`resources`):

- `vfsTotalQuotaBytes`
- `vfsMaxFileBytes`

Global CLI flags (`thunder start`, `thunder watch`):

- `--vfs-total-quota-bytes`
- `--vfs-max-file-bytes`

Global env vars:

- `EDGE_RUNTIME_VFS_TOTAL_QUOTA_BYTES`
- `EDGE_RUNTIME_VFS_MAX_FILE_BYTES`

## API Behavior Summary

Supported and VFS-backed:

- Sync: `readFileSync`, `writeFileSync`, `mkdirSync`, `readdirSync`, `statSync`, `lstatSync`, `existsSync`, `accessSync`
- Callback: `readFile`, `writeFile`, `mkdir`, `readdir`, `stat`, `lstat`
- Promises: `readFile`, `writeFile`, `mkdir`, `readdir`, `stat`, `lstat`

Not implemented in current VFS phase:

- `createReadStream`
- `createWriteStream`
- `watch`

These fail with deterministic `EOPNOTSUPP`.

## Error Model

Common deterministic errors:

- `ENOENT`: missing file/dir or missing parent directory
- `ENOTDIR`: `readdir` target is not a directory
- `EISDIR`: `readFile` target is a directory
- `EROFS`: write/mkdir attempt on read-only mount (`/bundle`)
- `ENOSPC`: per-file or total quota exceeded
- `EOPNOTSUPP`: operation not supported in current VFS phase or write outside writable mounts

## Lifecycle and Isolation

- VFS state is isolate-scoped and in-memory.
- `/tmp` is ephemeral by design.
- Data is not persisted to host disk.

## Practical Tuning Guidance

- Start with defaults (`10 MiB` total, `5 MiB` per-file).
- Increase cautiously for workloads that build temp artifacts.
- Keep limits explicit; do not bind quotas directly to GC heap usage.

## Example Manifest Resource Block

```json
{
  "resources": {
    "vfsTotalQuotaBytes": 10485760,
    "vfsMaxFileBytes": 5242880
  }
}
```

## Example Runtime Flags

```bash
thunder start \
  --vfs-total-quota-bytes 10485760 \
  --vfs-max-file-bytes 5242880
```
