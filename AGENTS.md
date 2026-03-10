# Agent Guidelines

Instructions for AI agents working on this codebase.

---

## Project Overview

**Thunder** is a Rust-powered edge runtime built on the Deno stack. It executes user-provided JavaScript/TypeScript functions in isolated V8 sandboxes, exposed via an HTTP ingress server with an admin control API.

- **Language:** Rust (workspace with 4 crates) + TypeScript/JavaScript (Node.js compat polyfills, bootstrap scripts, test suites)
- **Runtime:** Tokio async runtime, Deno Core (V8 engine), Hyper/Tower HTTP stack
- **Binary:** `thunder` CLI with subcommands: `start`, `bundle`, `watch`, `test`, `check`
- **Bundle formats:** ESZIP and V8 snapshot
- **Deployment target:** Edge runtime server (Linux x86_64, macOS x86_64, macOS aarch64)
- **License:** MIT

---

## Quick Reference

### Commands

```bash
# Build
cargo build                       # Debug build
cargo build --release --locked    # Release build
make build                        # Same as cargo build
make release                      # Release build + copy binary to ./thunder

# Run
make run                          # Start server with default pool config
make run-latency                  # Start with latency-optimized profile
make run-throughput               # Start with throughput-optimized profile
make watch                        # Watch mode with hot-reload (hello example)

# Test
make test                         # Run Rust fast tests + JS tests
make test-full                    # Run full Rust test suite + JS tests
make test-rust-fast               # Rust tests only (skip E2E/stress, skip server E2E crate)
make test-rust-full               # Full Rust test suite
make test-js                      # JS/TS tests only via thunder test runner

# Cargo aliases (defined in .cargo/config.toml)
cargo test-dev                    # Fast loop: skip E2E/stress tests, exclude edge-server
cargo test-full                   # Full workspace test suite
cargo test-server-e2e             # Focused server E2E tests

# Lint / Format
cargo fmt                         # Format Rust code
cargo fmt --check                 # Check Rust formatting (CI)
cargo clippy --workspace          # Run Rust linter

# CLI usage (via cargo run or built binary)
cargo run -- start                # Start the runtime server
cargo run -- bundle --path <file> # Bundle a TS/JS file
cargo run -- watch --path <file>  # Watch mode
cargo run -- test --path <glob>   # Run JS/TS tests
cargo run -- check --path <file>  # Typecheck (delegates to deno check)
```

---

## Architecture

### Workspace Structure

```
deno-edge-runtime/
  Cargo.toml              # Workspace root
  Makefile                 # Build/run/test targets
  rust-toolchain.toml      # Rust stable + rustfmt + clippy
  .cargo/config.toml       # Cargo aliases, V8 mirror
  crates/
    runtime-core/          # V8/Deno isolate primitives, extensions, Node.js compat
    functions/             # Function lifecycle, registry, request handler
    server/                # HTTP server (Hyper/Tower), routing, admin API, TLS
    cli/                   # thunder binary, CLI commands (clap)
  examples/                # 29 sample edge functions (TS + pre-built eszips)
  tests/js/                # JS/TS test suites (run via thunder test)
  schemas/                 # JSON Schema definitions for function/routing manifests
  scripts/                 # Shell scripts (bundling, deployment, benchmarks)
  observability/           # Docker Compose observability stack (Grafana, Prometheus, etc.)
  docs/                    # Project documentation (18 markdown files)
```

### Crate Dependency Graph

```
cli (binary: thunder)
 ├── server (edge-server)
 │    ├── functions
 │    │    └── runtime-core
 │    └── runtime-core
 ├── functions
 └── runtime-core
```

### Crate Responsibilities

| Crate | Package Name | Role |
|-------|-------------|------|
| `crates/runtime-core` | `runtime-core` | V8 isolate management, Deno extension registration, Node.js polyfills (42 modules), permissions, SSRF protection, CPU/memory limits, JS bootstrap, testing assertion library |
| `crates/functions` | `functions` | Function registry/pool, isolate lifecycle (create/destroy/reuse), HTTP request handler, egress connection management, metrics, snapshot support |
| `crates/server` | `edge-server` | Hyper + Tower HTTP server, ingress routing, admin API (deploy/update/delete functions), global routing, TLS, body limits, bundle signature verification, middleware |
| `crates/cli` | `edge-cli` | `thunder` binary entry point, subcommands (`start`, `bundle`, `watch`, `test`, `check`), OpenTelemetry setup |

### Key Source Files by Size and Importance

| File | Lines | Role |
|------|-------|------|
| `crates/server/src/lib.rs` | ~3000+ | Server startup, connection management, listener loops |
| `crates/functions/src/registry.rs` | ~2500+ | Function registry, isolate pool management |
| `crates/functions/src/handler.rs` | ~2000+ | HTTP request-to-isolate dispatch |
| `crates/functions/src/lifecycle.rs` | ~1500+ | Isolate lifecycle (boot, eval, teardown) |
| `crates/server/src/router.rs` | ~1500+ | Request routing logic |
| `crates/runtime-core/src/manifest.rs` | ~900+ | Function manifest parsing and validation |
| `crates/runtime-core/src/extensions.rs` | ~700+ | Deno extension registration |
| `crates/runtime-core/src/bootstrap.js` | ~600+ | JavaScript bootstrap code injected into every isolate |

---

## Technology Stack

| Category | Technology | Version/Notes |
|----------|-----------|--------------|
| Language | Rust | Stable channel (see `rust-toolchain.toml`) |
| Async runtime | Tokio | 1.36+ (full features) |
| JS engine | Deno Core (`deno_core`) | 0.390.0 (wraps V8) |
| HTTP server | Hyper + Hyper-util | 1.4.1 (HTTP/1 + HTTP/2) |
| Middleware | Tower + Tower-HTTP | 0.4.13 / 0.6.1 |
| TLS | Rustls + Tokio-Rustls | 0.23.11 / 0.26.0 |
| CLI framework | Clap | 4.5.16 (derive API) |
| Serialization | Serde + Serde JSON | 1.0+ |
| Error handling | Anyhow | 1.0+ |
| Observability | OpenTelemetry (OTLP) + Tracing | 0.27.0 / 0.1 |
| Module bundling | ESZIP | 0.109.0 |
| Bundle signing | Ed25519 (ed25519-dalek) | In server crate |
| Node compat | `deno_node` + custom polyfills | 42 Node.js modules |

---

## Development Setup

### Prerequisites

- **Rust toolchain** (stable channel) -- installed via `rustup`, components defined in `rust-toolchain.toml`
- **Cargo** (comes with Rust)
- **Optional:** `deno` CLI for typechecking (`thunder check`)
- **Optional:** `k6` for load testing

### Build

```bash
cargo build           # Debug build
cargo build --release # Release build (LTO enabled)
make install          # Build release + install to /usr/local/bin/thunder
```

### First Run

```bash
# Start the server with default config
make run

# Or with cargo directly
cargo run -- start

# Watch mode for development (hot-reload on file changes)
make watch
```

---

## Testing

### Test Organization

| Category | Command | Location | Description |
|----------|---------|----------|-------------|
| Rust fast | `cargo test-dev` | `crates/*/src/`, `crates/functions/tests/` | Unit + integration tests, skipping E2E/stress and server E2E |
| Rust full | `cargo test-full` | All workspace crates | Complete Rust test suite including server E2E |
| Server E2E | `cargo test-server-e2e` | `crates/server/` | Focused server E2E tests |
| JS/TS | `make test-js` | `tests/js/*.test.ts` | JS test suites run via built-in `thunder test` runner |
| All (fast) | `make test` | Both | Rust fast + JS tests |
| All (full) | `make test-full` | Both | Rust full + JS tests |

### Running Specific Tests

```bash
# Specific Rust test by name
cargo test -p functions sandbox_blocks_private_fetch_targets

# Specific crate
cargo test -p runtime-core

# Specific JS test file
cargo run -- test --path ./tests/js/web_apis_full.test.ts

# JS tests with V8 inspector for debugging
cargo run -- test --path ./tests/js/web_apis_full.test.ts --inspect 9229
```

### Rust Test Patterns

Rust integration tests in `crates/functions/tests/` use two archetypes:

**Archetype A -- Lightweight JS execution:**
- Uses `#[test]` (not `#[tokio::test]`)
- Creates a `JsRuntime` directly via helper functions
- Tests JS API surface with `assert_js_true(js_code, description)` helper
- No network or isolate pool involved

**Archetype B -- Full isolate lifecycle:**
- Uses `#[test]` with manual `tokio::runtime::Builder::new_current_thread()` and `LocalSet`
- Builds eszip bundles inline, deploys to a `FunctionRegistry`
- Invokes via HTTP-like requests, asserts on status codes and body content
- Required because Deno's `JsRuntime` is `!Send`

**Important:** Each test file is self-contained. Helper functions (`make_runtime`, `build_eszip_async`, `deploy_inline_function`, `invoke_text`, `assert_js_true`) are duplicated across files -- there is no shared test utility module.

### JS/TS Test Patterns

JS tests use the built-in `thunder:testing` library (not Jest/Vitest/Deno test):

```typescript
import { runSuite, assert, assertEquals } from "thunder:testing";

await runSuite("suite-name", [
  {
    name: "test description",
    run: () => {
      assert(condition, "failure message");
    },
  },
]);
```

Available APIs: `assert`, `assertEquals`, `assertNotEquals`, `assertStrictEquals`, `assertExists`, `assertInstanceOf`, `assertMatch`, `assertRejects`, `assertThrows`, `assertArrayIncludes`, `assertObjectMatch`, `mockFn`, `spyOn`, `mockFetch`, `mockTime`, `runSuite`, `test`, `testIf`, `testEach`, `suite`, `beforeAll`, `afterAll`, `beforeEach`, `afterEach`.

### Test Naming Conventions

- **Rust functions:** `snake_case` descriptive names (e.g., `sandbox_blocks_private_fetch_targets`, `handler_fetch_pattern`)
- **JS suite names:** `kebab-case` (e.g., `"web-apis-full"`, `"mocking-system"`)
- **JS test names:** Natural English phrases (e.g., `"fetch primitives are available"`)
- **File names:** Rust: `snake_case.rs`; JS: `snake_case.test.ts`

---

## Code Conventions

### Rust Style

- **Formatting:** `rustfmt` (standard config, no `.rustfmt.toml` overrides)
- **Linting:** `clippy` (standard config, no suppression files)
- **Error handling:** `anyhow::Error` throughout; propagation via `?` and `map_err()`. No custom error enums. `thiserror` is declared but unused.
- **Async:** Tokio single-threaded runtime for code touching V8 (`!Send`), `tokio::task::LocalSet` required. Multi-threaded Tokio for server network I/O.
- **Logging:** `tracing` crate exclusively (`tracing::info!`, `tracing::warn!`, `tracing::error!`, `tracing::debug!`). No `log` crate usage.
- **CLI args:** `clap` derive API with `#[arg(long, env = "EDGE_RUNTIME_*")]` for dual CLI/env configuration.
- **Serialization:** `serde` derives with `#[serde(rename_all = "camelCase")]` or `#[serde(rename_all = "snake_case")]`.
- **Shared state:** `Arc<T>` for cross-task sharing, `OnceLock`/`Mutex` for process globals, `AtomicU64` for lock-free metrics.
- **Module organization:** Flat `pub mod` declarations in `lib.rs`. Each crate's `lib.rs` is minimal (except `server/src/lib.rs` which also contains server startup logic).

### TypeScript/JavaScript Style

- No external linter or formatter configured for TS/JS
- Node.js compatibility polyfills in `crates/runtime-core/src/node_compat/` (42 modules)
- Bootstrap scripts in `crates/runtime-core/src/` (`.js` files)
- Testing library in `crates/runtime-core/src/assert/`

---

## CI/CD Pipeline

Defined in `.github/workflows/ci-cd.yml`. Triggered on pushes to `main`, version tags (`v*`), and pull requests.

### Jobs

| Job | Trigger | Steps |
|-----|---------|-------|
| `build_ci` | All triggers | Checkout, Rust setup, sccache, `cargo build -p edge-cli --locked`, `cargo test --workspace --all-targets --no-run` |
| `test_ci` | After `build_ci` | `cargo test --workspace --all-targets`, `make test-js`, verify web standards report is up to date |
| `build_release_artifacts` | Main push / tags (after tests pass) | Multi-platform release build: linux-x86_64, macos-x86_64, macos-aarch64 |
| `release_unstable` | Main push | Publish pre-release to GitHub Releases (tag: `unstable`) |
| `release_stable` | Version tags (`v*`) | Publish stable release to GitHub Releases |

### CI Checks to Pass

Before merging, ensure:

1. `cargo test --workspace --all-targets` passes
2. `make test-js` passes
3. Web standards report is up to date: `cargo test -p functions --test web_api_report generate_web_standards_report -- --nocapture` produces no diff in `docs/web_standards_api_report.md`
4. Code compiles with `--locked` (no unintended dependency changes)

---

## Key Environment Variables

All configuration can be set via CLI flags or `EDGE_RUNTIME_*` environment variables. The most commonly needed:

| Variable | Default | Purpose |
|----------|---------|---------|
| `EDGE_RUNTIME_PORT` | `8080` | Ingress listen port |
| `EDGE_RUNTIME_ADMIN_PORT` | `9000` | Admin API port |
| `EDGE_RUNTIME_HOST` | `0.0.0.0` | Bind address |
| `EDGE_RUNTIME_POOL_MAX_ISOLATES` | `10` | Max isolates per function |
| `EDGE_RUNTIME_POOL_GLOBAL_MAX_ISOLATES` | `64` | Global max isolates |
| `EDGE_RUNTIME_MAX_HEAP_MIB` | `128` | Max V8 heap per isolate (MiB) |
| `EDGE_RUNTIME_CPU_TIME_LIMIT_MS` | `50000` | CPU time limit per request (ms) |
| `EDGE_RUNTIME_WALL_CLOCK_TIMEOUT_MS` | `60000` | Wall clock timeout per request (ms) |
| `EDGE_RUNTIME_LOG_FORMAT` | `pretty` | Log format (`pretty`/`json`) |
| `EDGE_RUNTIME_OTEL_ENABLED` | `false` | Enable OpenTelemetry export |
| `EDGE_RUNTIME_DISABLE_SSRF_PROTECTION` | `false` | Disable SSRF protection |
| `EDGE_RUNTIME_PRINT_ISOLATE_LOGS` | `true` | Print function console output |
| `RUST_LOG` | `info` | Rust tracing log level filter |

See the CLI `--help` output or `crates/cli/src/commands/start.rs` for the full list (~55 env vars).

---

## Important Considerations for Agents

### V8 and `!Send` Constraints

Deno's `JsRuntime` and V8 handles are **not `Send`**. All code that interacts with V8 must run on a `tokio::task::LocalSet` within a single-threaded Tokio runtime. This is the reason tests use `#[test]` with manual runtime construction instead of `#[tokio::test]`.

### Large Source Files

Several files exceed 1000 lines. When modifying these, read only the relevant sections:

- `crates/server/src/lib.rs` (~3000+ lines) -- server startup and connection management
- `crates/functions/src/registry.rs` (~2500+ lines) -- isolate pool/registry
- `crates/functions/src/handler.rs` (~2000+ lines) -- request handler
- `crates/functions/src/lifecycle.rs` (~1500+ lines) -- isolate lifecycle
- `crates/server/src/router.rs` (~1500+ lines) -- routing

### Bundle and Artifact Files

The `.gitignore` excludes `*.eszip`, `*.bundle`, and `target/`. However, some pre-built bundles exist at the repo root (`hello.eszip`, `rate-limiting.eszip`, etc.) and in `examples/`. Do not modify these binary files.

### No Package.json

This is a Rust workspace project. There is no `package.json`, `node_modules`, or JS package manager. TypeScript/JavaScript files are compiled into eszip bundles by the Rust toolchain and executed inside V8 isolates.

### Schemas

JSON Schema definitions for function manifests live in `schemas/`. When modifying manifest parsing in `crates/runtime-core/src/manifest.rs`, keep schemas in sync.

### Web Standards Report

The file `docs/web_standards_api_report.md` is auto-generated by a test. Do not edit it manually. It is regenerated by running:
```bash
cargo test -p functions --test web_api_report generate_web_standards_report -- --nocapture
```
CI verifies this file has no uncommitted changes.
