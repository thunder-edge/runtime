# Deno Edge Runtime CLI

This document describes the `edge-cli` crate command-line interface exposed by the `deno-edge-runtime` binary.

## Binary

- Crate: `crates/cli` (`edge-cli`)
- Executable name: `deno-edge-runtime`
- Primary purpose: run, bundle, watch, test, and typecheck JavaScript/TypeScript edge functions.

## Quick Start

Build:

```bash
cargo build
```

Run the server (default host/port):

```bash
cargo run -- start
```

Run JS/TS runtime tests:

```bash
cargo run -- test --path "./tests/js/**/*.ts" --ignore "./tests/js/lib/**"
```

Typecheck sources:

```bash
cargo run -- check --path "./**/*.{ts,js,mts,mjs,tsx,jsx,cjs,cts}"
```

Bundle one entrypoint into ESZIP package:

```bash
cargo run -- bundle --entrypoint ./examples/hello/hello.ts --output ./hello.eszip
```

## Global CLI Syntax

```bash
deno-edge-runtime [GLOBAL_OPTIONS] <COMMAND> [COMMAND_OPTIONS]
```

### Global Options

- `-v, --verbose`
  - Enables debug logging (`RUST_LOG=debug` behavior unless overridden by environment).

### Subcommands

- `start` - Start the edge runtime server.
- `bundle` - Bundle a JS/TS entrypoint into a package.
- `watch` - Watch files and continuously deploy functions on changes.
- `test` - Run JS/TS compatibility/runtime tests.
- `check` - Typecheck or syntax-validate JS/TS files.

## Common Environment Variables

These are consumed mainly by `start` and `watch`:

- `EDGE_RUNTIME_HOST`
- `EDGE_RUNTIME_PORT`
- `EDGE_RUNTIME_MAX_HEAP_MIB`
- `EDGE_RUNTIME_CPU_TIME_LIMIT_MS`
- `EDGE_RUNTIME_WALL_CLOCK_TIMEOUT_MS`

`start` also supports:

- `EDGE_RUNTIME_TLS_CERT`
- `EDGE_RUNTIME_TLS_KEY`
- `EDGE_RUNTIME_RATE_LIMIT`
- `EDGE_RUNTIME_SOURCE_MAP`

## Command Reference

## `start`

Starts the HTTP runtime server and waits for shutdown signal.

### Usage

```bash
deno-edge-runtime start [OPTIONS]
```

### Options

- `--host <HOST>`
  - Default: `0.0.0.0`
  - Env: `EDGE_RUNTIME_HOST`
- `-p, --port <PORT>`
  - Default: `9000`
  - Env: `EDGE_RUNTIME_PORT`
- `--tls-cert <TLS_CERT>`
  - Env: `EDGE_RUNTIME_TLS_CERT`
  - Requires `--tls-key` to effectively enable TLS.
- `--tls-key <TLS_KEY>`
  - Env: `EDGE_RUNTIME_TLS_KEY`
  - Requires `--tls-cert` to effectively enable TLS.
- `--rate-limit <RATE_LIMIT>`
  - Requests per second.
  - Default: `0` (unlimited)
  - Env: `EDGE_RUNTIME_RATE_LIMIT`
- `--graceful-exit-timeout <GRACEFUL_EXIT_TIMEOUT>`
  - Graceful shutdown deadline in seconds.
  - Default: `30`
- `--max-heap-mib <MAX_HEAP_MIB>`
  - Per-isolate heap limit in MiB.
  - Default: `128`
  - `0` means unlimited.
  - Env: `EDGE_RUNTIME_MAX_HEAP_MIB`
- `--cpu-time-limit-ms <CPU_TIME_LIMIT_MS>`
  - Per-request CPU limit.
  - Default: `50000`
  - `0` means unlimited.
  - Env: `EDGE_RUNTIME_CPU_TIME_LIMIT_MS`
- `--wall-clock-timeout-ms <WALL_CLOCK_TIMEOUT_MS>`
  - Per-request wall clock timeout.
  - Default: `60000`
  - `0` means unlimited.
  - Env: `EDGE_RUNTIME_WALL_CLOCK_TIMEOUT_MS`
- `--sourcemap <SOURCEMAP>`
  - Source map handling for modules loaded from ESZIP.
  - Values: `none`, `inline`
  - Default: `none`
  - Env: `EDGE_RUNTIME_SOURCE_MAP`

### Behavior Notes

- Installs Rustls ring provider at startup for TLS operations.
- Initializes V8 platform on main thread before creating runtimes.
- If only one of `--tls-cert` or `--tls-key` is provided, TLS is not enabled.
- On shutdown, the server stops and all deployed functions are shut down.

### Example

```bash
deno-edge-runtime start \
  --host 0.0.0.0 \
  --port 9000 \
  --max-heap-mib 256 \
  --cpu-time-limit-ms 10000 \
  --wall-clock-timeout-ms 15000 \
  --rate-limit 200
```

### TLS Configuration

The server supports HTTPS via TLS certificates. When both `--tls-cert` and `--tls-key` are provided, the server serves HTTPS instead of HTTP.

#### Generating a Self-Signed Certificate (Development)

```bash
openssl req -x509 -newkey rsa:4096 \
  -keyout key.pem \
  -out cert.pem \
  -days 365 \
  -nodes \
  -subj '/CN=localhost'
```

#### Starting with TLS

```bash
deno-edge-runtime start \
  --tls-cert cert.pem \
  --tls-key key.pem \
  --port 9443
```

Or using environment variables:

```bash
export EDGE_RUNTIME_TLS_CERT=./certs/server.crt
export EDGE_RUNTIME_TLS_KEY=./certs/server.key
deno-edge-runtime start --port 9443
```

#### Testing HTTPS

```bash
# With self-signed certificate (skip verification)
curl -k https://localhost:9443/_internal/metrics

# With CA-signed certificate
curl https://your-domain.com:9443/_internal/metrics
```

#### TLS Notes

- **ALPN**: The server advertises both `h2` (HTTP/2) and `http/1.1` protocols via ALPN.
- **Startup log**: When TLS is enabled, the startup log shows `https://` instead of `http://`.
- **Handshake failures**: Failed TLS handshakes are logged at `warn` level with the client address.
- **Production**: For production, use certificates signed by a trusted CA (e.g., Let's Encrypt).

## `bundle`

Bundles a JS/TS entrypoint and dependencies into a serialized package (ESZIP package format).

### Usage

```bash
deno-edge-runtime bundle --entrypoint <FILE> --output <FILE> [OPTIONS]
```

### Options

- `-e, --entrypoint <ENTRYPOINT>`
  - Required.
  - Entry JS/TS file.
- `-o, --output <OUTPUT>`
  - Required.
  - Destination file path.
- `-f, --format <FORMAT>`
  - Default: `eszip`
  - Accepted values: `eszip`, `snapshot`

### Behavior Notes

- For TS-like entrypoints (`.ts`, `.mts`, `.cts`, `.tsx`):
  - If `deno` is available in `PATH`, runs `deno check` first.
  - If not available, falls back to syntax/module-graph validation only.
- `snapshot` value is currently rejected at runtime with an explicit error.
- Result is written as a serialized `BundlePackage` containing ESZIP bytes.
- Supports `edge://assert/*` imports through internal rewrite/loader handling.

### Examples

```bash
deno-edge-runtime bundle \
  --entrypoint ./examples/json-api/json-api.ts \
  --output ./bundles/eszip/json-api.eszip
```

Expected error for unsupported format:

```bash
deno-edge-runtime bundle \
  --entrypoint ./examples/hello/hello.ts \
  --output ./out.bin \
  --format snapshot
```

## `watch`

Watches a path for file changes, bundles discovered JS/TS files, and deploys/updates functions live.

### Usage

```bash
deno-edge-runtime watch [OPTIONS]
```

### Options

- `--path <PATH>`
  - Default: `.`
  - Directory to scan/watch.
- `--host <HOST>`
  - Default: `0.0.0.0`
  - Env: `EDGE_RUNTIME_HOST`
- `-p, --port <PORT>`
  - Default: `9000`
  - Env: `EDGE_RUNTIME_PORT`
- `--interval <INTERVAL>`
  - Debounce in milliseconds for reload after file changes.
  - Default: `1000`
- `--max-heap-mib <MAX_HEAP_MIB>`
  - Default: `128`
  - `0` means unlimited.
  - Env: `EDGE_RUNTIME_MAX_HEAP_MIB`
- `--cpu-time-limit-ms <CPU_TIME_LIMIT_MS>`
  - Default: `50000`
  - `0` means unlimited.
  - Env: `EDGE_RUNTIME_CPU_TIME_LIMIT_MS`
- `--wall-clock-timeout-ms <WALL_CLOCK_TIMEOUT_MS>`
  - Default: `60000`
  - `0` means unlimited.
  - Env: `EDGE_RUNTIME_WALL_CLOCK_TIMEOUT_MS`
- `--inspect [PORT]`
  - Optional base port for V8 inspector in watch mode.
  - If provided without value, default is `9229`.
  - For multiple functions, ports auto-increment (`base`, `base+1`, ...).
- `--inspect-brk`
  - Break on first statement while waiting for debugger attach.
  - Requires `--inspect` to be useful.

### Behavior Notes

- `watch` rejects non-existing `--path`.
- Scans recursively and only considers files ending in `.ts` or `.js`.
- Skips files under directories named:
  - `node_modules`, `dist`, `build`, `.next`, `.deno`, `target`
- Converts file paths to function names by joining path segments with `-` and removing extension.
- First deployment uses `deploy`; existing function names are updated with `update`.
- Server in watch mode uses immediate shutdown behavior (`graceful_exit_deadline_secs = 0`).

### Example

```bash
deno-edge-runtime watch \
  --path ./examples \
  --port 9000 \
  --interval 500 \
  --inspect 9230
```

## `test`

Runs JS/TS runtime test files in an isolated runtime, with optional debugger inspector support.

### Usage

```bash
deno-edge-runtime test [OPTIONS]
```

### Options

- `-p, --path <PATH>`
  - Path, directory, or glob pattern.
  - Default: `./tests/js/**/*.ts`
- `-i, --ignore <IGNORE>`
  - Ignore path/pattern.
  - Can be repeated.
- `--inspect [PORT]`
  - Enable inspector protocol server.
  - If provided without value, defaults to `9229`.

### Behavior Notes

- Supported test file extensions:
  - `.ts`, `.js`, `.mts`, `.mjs`
- Input discovery supports:
  - explicit file,
  - directory walk,
  - glob pattern.
- If no files are found, command exits with error.
- If `--inspect` is used, exactly one test file must be selected.
- During execution, the command prints per-file result (`PASS`/`FAIL`), progress bar, and aggregated test stats.
- Inspector HTTP endpoints when enabled:
  - `GET /json`
  - `GET /json/list`
  - `GET /json/version`
  - WebSocket endpoint: `/ws`

### Examples

Run all tests except helper library files:

```bash
deno-edge-runtime test \
  --path "./tests/js/**/*.ts" \
  --ignore "./tests/js/lib/**"
```

Debug a single test file:

```bash
deno-edge-runtime test --path ./tests/js/my_test.ts --inspect 9229
```

## `check`

Typechecks source files with `deno check`, or falls back to syntax/module validation when Deno is unavailable.

### Usage

```bash
deno-edge-runtime check [OPTIONS]
```

### Options

- `-p, --path <PATH>`
  - Path, directory, or glob pattern.
  - Default: `./**/*.{ts,js,mts,mjs,tsx,jsx,cjs,cts}`
- `-i, --ignore <IGNORE>`
  - Ignore path/pattern.
  - Can be repeated.

### Behavior Notes

- Supported source extensions:
  - `.ts`, `.js`, `.mts`, `.mjs`, `.tsx`, `.jsx`, `.cjs`, `.cts`
- If `deno` binary is found in `PATH`:
  - Executes `deno check <files...>`.
- If `deno` is not found:
  - Falls back to syntax/module-graph validation using internal graph builder.
- Fails if no matching source files are found.

### Examples

```bash
deno-edge-runtime check --path ./examples
```

```bash
deno-edge-runtime check \
  --path "./**/*.{ts,tsx}" \
  --ignore "./target/**" \
  --ignore "./node_modules/**"
```

## Exit Behavior

- Commands return non-zero exit code on error.
- Typical failure causes:
  - invalid or missing paths,
  - module graph/build failures,
  - bundling/deployment errors,
  - test failures,
  - failed `deno check`.

## Troubleshooting

### `deno` is not installed

Symptoms:

- `check` or `bundle` prints warning about missing `deno` binary.

Impact:

- Semantic TS typechecking is skipped.
- Only syntax/module validation is performed.

Fix:

```bash
brew install deno
```

### `watch` is not updating functions

Checks:

- Verify `--path` points to the intended source root.
- Confirm files are `.ts` or `.js`.
- Ensure files are not inside skipped directories (`node_modules`, `dist`, `target`, etc.).
- Reduce debounce with `--interval 200` for faster feedback.

### Inspector does not connect

Checks:

- Ensure port is free.
- In `test`, ensure only one file is selected with `--inspect`.
- Use the endpoint `http://127.0.0.1:<PORT>/json/list` to discover targets.

## Additional Notes

- The CLI initializes tracing and V8 platform before dispatching subcommands.
- Logs default to `info` level unless `--verbose` is used or `RUST_LOG` is explicitly configured.
- Current snapshot bundling flag is intentionally documented but not functional yet.
