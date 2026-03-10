# Node Compatibility Matrix

> Canonical compatibility model for `node:*` APIs in this runtime profile.
> Last updated: 2026-03-08.

## Levels

- `Full`: implementation is functionally complete for the tested contract.
- `Partial`: implementation is functional for common/runtime-safe paths with documented limitations.
- `Stub`: module is importable, but unsupported methods fail deterministically when called.

Stub error format:

`[thunder] <api> is not implemented in this runtime profile`

## Module Status

| Module | Level | Notes |
|---|---|---|
| `node:assert` | Partial | Assertion helpers for common compatibility flows. |
| `node:async_hooks` | Partial | `AsyncLocalStorage` and base hook propagation for common async boundaries. |
| `node:buffer` | Partial | Common Buffer operations used by SSR/tooling. |
| `node:child_process` | Stub | Process-spawn APIs are importable stubs with deterministic failures. |
| `node:cluster` | Stub | Cluster APIs are importable stubs with deterministic failures. |
| `node:console` | Partial | Console module mapped to runtime console implementation. |
| `node:crypto` | Partial | `randomBytes`/`randomFill` + `createHash`/`createHmac` subset. |
| `node:dgram` | Stub | UDP socket APIs are importable stubs with deterministic failures. |
| `node:diagnostics_channel` | Partial | Pub/sub + `TracingChannel` subset. |
| `node:dns` | Partial | DoH-backed subset (`lookup`, `resolve*`, `reverse`) with limits. |
| `node:events` | Partial | EventEmitter-compatible surface for common listener flows. |
| `node:fs` | Partial | VFS-backed behavior (`/bundle` read-only, `/tmp` writable, `/dev/null`) with `createReadStream`/`createWriteStream` support. |
| `node:fs/promises` | Partial | Promise APIs aligned with VFS behavior and deterministic errors. |
| `node:http` | Partial | Client compatibility via `fetch` wrapper; `createServer` is importable and `Server.listen` fails deterministically by sandbox policy. |
| `node:http2` | Stub | Importable compatibility surface with deterministic non-functional operations. |
| `node:https` | Partial | Client compatibility via `fetch` wrapper; server APIs remain non-functional. |
| `node:inspector` | Stub | Importable inspector surface with deterministic non-functional operations. |
| `node:module` | Partial | `createRequire` with built-in-only deterministic resolution policy. |
| `node:net` | Partial | Outbound client subset; `net.Server` methods remain deterministic stubs. |
| `node:os` | Partial | Contract-stable host info with deterministic errors for blocked operations. |
| `node:path` | Partial | Deterministic path helper subset for module/tooling compatibility. |
| `node:perf_hooks` | Partial | Performance hooks compatibility on runtime performance APIs. |
| `node:process` | Partial | Sandboxed process subset with local `env` and virtual cwd. |
| `node:punycode` | Partial | Punycode helper subset. |
| `node:querystring` | Partial | Parse/stringify compatibility helpers. |
| `node:readline` | Stub | Importable interactive APIs with deterministic non-functional behavior. |
| `node:repl` | Stub | Importable REPL APIs with deterministic non-functional behavior. |
| `node:request` | Partial | Request helper adapter over HTTP compat wrapper. |
| `node:sqlite` | Stub | Importable module; constructors fail deterministically. |
| `node:stream` | Partial | Stream primitives/pipeline subset for compatibility paths. |
| `node:string_decoder` | Partial | String decoder compatibility subset. |
| `node:test` | Stub | Importable module; test APIs fail deterministically when called. |
| `node:timers` | Partial | Timer APIs backed by runtime timer globals. |
| `node:timers/promises` | Partial | Promise-based timers subset. |
| `node:tls` | Partial | Outbound TLS client subset; server/context APIs remain stubs. |
| `node:url` | Partial | URL constructors, helpers, and domain ASCII/Unicode helpers. |
| `node:util` | Partial | Utility subset (`format`, `inspect`, `promisify`, `types`, `MIMEType`). |
| `node:v8` | Partial | Introspection helper subset with deterministic/static behavior. |
| `node:vm` | Stub | Importable VM APIs with deterministic non-functional behavior. |
| `node:worker_threads` | Stub | Importable module for feature detection; worker spawning APIs fail deterministically due to sandbox thread restrictions. |
| `node:zlib` | Partial | Functional one-shot gzip/deflate/inflate subset with guardrails. |

## Validation

- Source of truth for runtime assertions: `crates/functions/tests/web_api_report.rs` (`define_node_compat_checks`).
- Generated runtime report: `docs/web_standards_api_report.md`.
