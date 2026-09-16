# Agent Guidelines

Thunder is a public MIT-licensed Rust edge runtime built on `deno_core` and V8. Keep every change
generic and vendor-neutral; never add organization-specific names, hosts, infrastructure, data,
credentials, tickets, or business rules. Express deployment-specific needs through manifests,
environment variables, CLI flags, or reusable extension points.

## Architecture

| Crate | Responsibility |
|---|---|
| `runtime-core` | V8 isolates, Deno extensions, Node compatibility, permissions and limits |
| `functions` | registry, isolate lifecycle and pool, request execution, egress and metrics |
| `server` | Hyper/Tower ingress, admin API, routing, TLS and body/signature enforcement |
| `cli` | `thunder` binary, commands and OpenTelemetry setup |

Dependency direction: `cli` → `server` → `functions` → `runtime-core`.

## Invariants

- Treat user JavaScript and bundles as untrusted input. Security boundaries require regression tests.
- `JsRuntime` and V8 handles are not `Send`; run V8 work on a single-thread Tokio runtime with
  `tokio::task::LocalSet`.
- Keep `schemas/` synchronized with parsing changes in `crates/runtime-core/src/manifest.rs`.
- Use `anyhow` for errors and `tracing` for logs, following nearby code.
- There is no Node package workflow. JavaScript and TypeScript are bundled and executed by the Rust
  toolchain.
- Do not edit generated `*.eszip`, `*.bundle`, or web standards report output by hand.
- Changes to `deno_*`, V8, or ESZIP versions must be coordinated across the stack.

## Testing

- Add or update a regression test for every behavior change, including failure and cleanup paths.
- Tests touching V8 generally use `#[test]`, a current-thread Tokio runtime, and `LocalSet`; do not
  convert them to `#[tokio::test]` unless the tested path is `Send`.
- Prefer the nearest existing test harness and run the narrowest relevant test first.

## Gates

```bash
cargo fmt --check
cargo clippy --workspace --all-targets
make test
cargo build --workspace --locked
```

Use `make test-full` for broad runtime changes. Run the relevant benchmark when changing startup,
latency, throughput, memory, pool behavior, or file-descriptor handling.
