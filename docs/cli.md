# Thunder Edge Runtime CLI

This document describes the `edge-cli` crate command-line interface exposed by the `thunder` binary.

Related docs:

- [Function Manifest (v2)](./function-manifest.md)
- [Virtual File System (VFS)](./vfs.md)

## Binary

- Crate: `crates/cli` (`edge-cli`)
- Executable name: `thunder`
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

Bundle with explicit format (`eszip` or `snapshot` envelope):

```bash
cargo run -- bundle \
  --entrypoint ./examples/hello/hello.ts \
  --output ./hello.snapshot.bundle \
  --format snapshot
```

## Global CLI Syntax

```bash
thunder [GLOBAL_OPTIONS] <COMMAND> [COMMAND_OPTIONS]
```

### Global Options

- `-v, --verbose`
  - Enables debug logging (`RUST_LOG=debug` behavior unless overridden by environment).
- `--log-format <pretty|json>`
  - Runtime log output format.
  - Default: `pretty`
  - Env: `EDGE_RUNTIME_LOG_FORMAT`
  - `watch` keeps the same default (`pretty`) unless overridden.
- `--otel-enabled`
  - Enable OpenTelemetry export for traces/metrics/logs.
  - Default: `false`
  - Env: `EDGE_RUNTIME_OTEL_ENABLED`
- `--otel-protocol <http-protobuf>`
  - OTLP transport protocol (currently HTTP protobuf).
  - Default: `http-protobuf`
  - Env: `EDGE_RUNTIME_OTEL_PROTOCOL`
- `--otel-endpoint <URL>`
  - OTLP collector base endpoint.
  - Default: `http://127.0.0.1:4318`
  - Env: `EDGE_RUNTIME_OTEL_ENDPOINT`
- `--otel-service-name <NAME>`
  - OTEL resource `service.name`.
  - Default: `thunder`
  - Env: `EDGE_RUNTIME_OTEL_SERVICE_NAME`
- `--otel-export-interval-ms <MS>`
  - Periodic export interval for OTEL batch readers.
  - Default: `5000`
  - Env: `EDGE_RUNTIME_OTEL_EXPORT_INTERVAL_MS`
- `--otel-export-timeout-ms <MS>`
  - OTEL export timeout.
  - Default: `10000`
  - Env: `EDGE_RUNTIME_OTEL_EXPORT_TIMEOUT_MS`
- `--otel-enable-traces`
  - Enable OTEL trace signal export.
  - Default: `true`
  - Env: `EDGE_RUNTIME_OTEL_ENABLE_TRACES`
- `--otel-enable-metrics`
  - Enable OTEL metrics signal export.
  - Default: `true`
  - Env: `EDGE_RUNTIME_OTEL_ENABLE_METRICS`
- `--otel-enable-logs`
  - Enable OTEL logs signal export.
  - Default: `true`
  - Env: `EDGE_RUNTIME_OTEL_ENABLE_LOGS`
- `--otel-export-isolate-logs`
  - Export isolate collector logs to OTEL logs signal.
  - Default: `true`
  - Env: `EDGE_RUNTIME_OTEL_EXPORT_ISOLATE_LOGS`
- `--otel-isolate-log-batch-size <COUNT>`
  - Max isolate logs exported per OTEL drain tick.
  - Default: `256`
  - Env: `EDGE_RUNTIME_OTEL_ISOLATE_LOG_BATCH_SIZE`

### Subcommands

- `start` - Start the edge runtime server.
- `bundle` - Bundle a JS/TS entrypoint into a package.
- `watch` - Watch files and continuously deploy functions on changes.
- `test` - Run JS/TS compatibility/runtime tests.
- `check` - Typecheck or syntax-validate JS/TS files.

## Common Environment Variables

These are consumed mainly by `start` and `watch`:

**Admin Listener:**
- `EDGE_RUNTIME_ADMIN_HOST`
- `EDGE_RUNTIME_ADMIN_PORT`
- `EDGE_RUNTIME_API_KEY`
- `EDGE_RUNTIME_ADMIN_TLS_CERT`
- `EDGE_RUNTIME_ADMIN_TLS_KEY`
- `EDGE_RUNTIME_REQUIRE_BUNDLE_SIGNATURE`
- `EDGE_RUNTIME_BUNDLE_PUBLIC_KEY_PATH`

**Ingress Listener:**
- `EDGE_RUNTIME_HOST`
- `EDGE_RUNTIME_PORT`
- `EDGE_RUNTIME_UNIX_SOCKET`
- `EDGE_RUNTIME_TLS_CERT`
- `EDGE_RUNTIME_TLS_KEY`

**Isolate Configuration:**
- `EDGE_RUNTIME_MAX_HEAP_MIB`
- `EDGE_RUNTIME_CPU_TIME_LIMIT_MS`
- `EDGE_RUNTIME_WALL_CLOCK_TIMEOUT_MS`
- `EDGE_RUNTIME_VFS_TOTAL_QUOTA_BYTES`
- `EDGE_RUNTIME_VFS_MAX_FILE_BYTES`

**Security Options:**
- `EDGE_RUNTIME_DISABLE_SSRF_PROTECTION`
- `EDGE_RUNTIME_ALLOW_PRIVATE_NET`

**Outgoing Proxy:**
- Runtime-scoped (process-wide): applies to all function isolates in the running runtime.
- `EDGE_RUNTIME_HTTP_OUTGOING_PROXY`
- `EDGE_RUNTIME_HTTPS_OUTGOING_PROXY`
- `EDGE_RUNTIME_TCP_OUTGOING_PROXY`
- `EDGE_RUNTIME_HTTP_NO_PROXY`
- `EDGE_RUNTIME_HTTPS_NO_PROXY`
- `EDGE_RUNTIME_TCP_NO_PROXY`

**Body Size Limits:**
- `EDGE_RUNTIME_MAX_REQUEST_BODY_SIZE`
- `EDGE_RUNTIME_MAX_RESPONSE_BODY_SIZE`

**Connection Limits:**
- `EDGE_RUNTIME_MAX_CONNECTIONS`

**Other:**
- `EDGE_RUNTIME_RATE_LIMIT`
- `EDGE_RUNTIME_SOURCE_MAP`
- `EDGE_RUNTIME_LOG_FORMAT`
- `EDGE_RUNTIME_PRINT_ISOLATE_LOGS`
- `EDGE_RUNTIME_OTEL_ENABLED`
- `EDGE_RUNTIME_OTEL_PROTOCOL`
- `EDGE_RUNTIME_OTEL_ENDPOINT`
- `EDGE_RUNTIME_OTEL_SERVICE_NAME`
- `EDGE_RUNTIME_OTEL_EXPORT_INTERVAL_MS`
- `EDGE_RUNTIME_OTEL_EXPORT_TIMEOUT_MS`
- `EDGE_RUNTIME_OTEL_ENABLE_TRACES`
- `EDGE_RUNTIME_OTEL_ENABLE_METRICS`
- `EDGE_RUNTIME_OTEL_ENABLE_LOGS`
- `EDGE_RUNTIME_OTEL_EXPORT_ISOLATE_LOGS`
- `EDGE_RUNTIME_OTEL_ISOLATE_LOG_BATCH_SIZE`

## Command Reference

## `bundle`

Bundles a JS/TS entrypoint into a deployable package.

### Usage

```bash
thunder bundle --entrypoint <FILE> --output <FILE> [--format <eszip|snapshot>]
```

### Options

- `-e, --entrypoint <FILE>`
  - Entrypoint JS/TS file.
- `-o, --output <FILE>`
  - Output bundle file path.
- `--format <eszip|snapshot>`
  - Default: `eszip`
  - `eszip`: writes a standard ESZIP package.
  - `snapshot`: writes a snapshot envelope with embedded ESZIP fallback.

Snapshot note:

- Snapshot output is packaged with ESZIP fallback for availability.
- If snapshot execution is unavailable or incompatible at runtime, startup falls back to ESZIP automatically.
- In V8 version mismatch cases, regenerate snapshot bundles with the current runtime V8.

## `start`

Starts the HTTP runtime server with a **dual-listener architecture**:

- **Admin Listener** (default port 9000): Handles `/_internal/*` management endpoints with optional API key authentication.
- **Ingress Listener** (default port 8080 or Unix socket): Handles function invocation requests (`/{function_name}/*`) without authentication.

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                     thunder                           │
│                                                                 │
│  ┌─────────────────────┐     ┌─────────────────────────────┐   │
│  │   Admin Listener    │     │     Ingress Listener        │   │
│  │   (port 9000)       │     │  (port 8080 or Unix socket) │   │
│  │                     │     │                             │   │
│  │  /_internal/health  │     │  /{function_name}/*         │   │
│  │  /_internal/metrics │     │                             │   │
│  │  /_internal/funcs   │     │  Public - No authentication │   │
│  │                     │     │                             │   │
│  │  X-API-Key required │     │                             │   │
│  └─────────────────────┘     └─────────────────────────────┘   │
│              │                           │                      │
│              └───────────┬───────────────┘                      │
│                          │                                      │
│                 ┌────────▼────────┐                            │
│                 │ FunctionRegistry│                            │
│                 │  (shared state) │                            │
│                 └─────────────────┘                            │
└─────────────────────────────────────────────────────────────────┘
```

### Usage

```bash
thunder start [OPTIONS]
```

### Admin Listener Options

- `--admin-host <HOST>`
  - Default: `0.0.0.0`
  - Env: `EDGE_RUNTIME_ADMIN_HOST`
- `--admin-port <PORT>`
  - Default: `9000`
  - Env: `EDGE_RUNTIME_ADMIN_PORT`
- `--api-key <API_KEY>`
  - API key for authentication on `/_internal/*` endpoints.
  - If not set, admin endpoints are open (dev mode) with a warning.
  - **Required for production use.**
  - Env: `EDGE_RUNTIME_API_KEY`
- `--admin-tls-cert <PATH>`
  - TLS certificate file path for admin listener.
  - Env: `EDGE_RUNTIME_ADMIN_TLS_CERT`
- `--admin-tls-key <PATH>`
  - TLS private key file path for admin listener.
  - Env: `EDGE_RUNTIME_ADMIN_TLS_KEY`

### Ingress Listener Options

- `--host <HOST>`
  - Default: `0.0.0.0`
  - Env: `EDGE_RUNTIME_HOST`
- `-p, --port <PORT>`
  - TCP port for ingress listener.
  - Default: `8080` (if neither `--port` nor `--unix-socket` is specified)
  - Mutually exclusive with `--unix-socket`.
  - Env: `EDGE_RUNTIME_PORT`
- `--unix-socket <PATH>`
  - Unix socket path for ingress listener.
  - Mutually exclusive with `--port`.
  - TLS options are ignored when using Unix socket.
  - Env: `EDGE_RUNTIME_UNIX_SOCKET`
- `--tls-cert <PATH>`
  - TLS certificate file path for ingress listener (TCP only).
  - Env: `EDGE_RUNTIME_TLS_CERT`
- `--tls-key <PATH>`
  - TLS private key file path for ingress listener (TCP only).
  - Env: `EDGE_RUNTIME_TLS_KEY`

### Security Options

- `--disable-ssrf-protection`
  - Disable SSRF protection, allowing `fetch()` to access private IP ranges.
  - Default: `false` (SSRF protection enabled)
  - **Not recommended for production.**
  - Env: `EDGE_RUNTIME_DISABLE_SSRF_PROTECTION`
- `--allow-private-net <CIDR,...>`
  - Allow specific private subnets despite SSRF protection.
  - Comma-separated list of CIDR ranges.
  - Example: `--allow-private-net "10.1.0.0/16,10.2.0.0/16"`
  - Useful for corporate networks or internal services.
  - Env: `EDGE_RUNTIME_ALLOW_PRIVATE_NET`

- `--http-outgoing-proxy <URL>`
  - Outgoing proxy for HTTP requests.
  - Examples: `http://proxy.local:8080`, `socks5://proxy.local:1080`
  - Env: `EDGE_RUNTIME_HTTP_OUTGOING_PROXY`
- `--https-outgoing-proxy <URL>`
  - Outgoing proxy for HTTPS requests.
  - Examples: `http://proxy.local:8080`, `socks5://proxy.local:1080`
  - Env: `EDGE_RUNTIME_HTTPS_OUTGOING_PROXY`
- `--tcp-outgoing-proxy <HOST:PORT|tcp://HOST:PORT>`
  - Generic TCP transport proxy used for outgoing egress.
  - Env: `EDGE_RUNTIME_TCP_OUTGOING_PROXY`
- `--http-no-proxy <HOST,...>`
  - Comma-separated bypass list for HTTP proxy.
  - Env: `EDGE_RUNTIME_HTTP_NO_PROXY`
- `--https-no-proxy <HOST,...>`
  - Comma-separated bypass list for HTTPS proxy.
  - Env: `EDGE_RUNTIME_HTTPS_NO_PROXY`
- `--tcp-no-proxy <HOST,...>`
  - Comma-separated bypass list for TCP proxy.
  - Env: `EDGE_RUNTIME_TCP_NO_PROXY`

- `--require-bundle-signature`
  - Require Ed25519 signature verification for function deploy/update payloads on admin API.
  - Default: `false`
  - When enabled, requests must include `x-bundle-signature-ed25519`.
  - Env: `EDGE_RUNTIME_REQUIRE_BUNDLE_SIGNATURE`
- `--bundle-public-key-path <PATH>`
  - Path to Ed25519 public key used to verify bundle signatures.
  - Accepted formats: PEM, base64 raw key (32 bytes), or hex raw key (32 bytes).
  - Required when `--require-bundle-signature` is enabled.
  - Env: `EDGE_RUNTIME_BUNDLE_PUBLIC_KEY_PATH`

#### SSRF Protection Details

When enabled (default), SSRF protection blocks `fetch()` requests to the following private IP ranges:

| Range | Description |
|-------|-------------|
| `127.0.0.0/8` | Loopback addresses |
| `10.0.0.0/8` | RFC 1918 private network |
| `172.16.0.0/12` | RFC 1918 private network |
| `192.168.0.0/16` | RFC 1918 private network |
| `169.254.0.0/16` | Link-local / Cloud metadata (e.g., AWS, GCP, Azure) |
| `0.0.0.0/8` | Reserved |
| `::1/128` | IPv6 loopback |
| `fc00::/7` | IPv6 unique local |
| `fe80::/10` | IPv6 link-local |

This prevents functions from accessing sensitive internal endpoints like cloud metadata services (`http://169.254.169.254/`), internal APIs, or localhost services.

### Body Size Limits

- `--max-request-body-size <BYTES>`
  - Maximum request body size in bytes.
  - Default: `5242880` (5 MiB)
  - Requests exceeding this limit receive `413 Payload Too Large`.
  - Env: `EDGE_RUNTIME_MAX_REQUEST_BODY_SIZE`
- `--max-response-body-size <BYTES>`
  - Maximum response body size in bytes.
  - Default: `10485760` (10 MiB)
  - Responses exceeding this limit return an error.
  - Env: `EDGE_RUNTIME_MAX_RESPONSE_BODY_SIZE`

### Connection Limits

- `--max-connections <COUNT>`
  - Maximum concurrent connections across all listeners.
  - Default: `10000`
  - Connections exceeding this limit are dropped immediately.
  - Protects against resource exhaustion attacks.
  - Env: `EDGE_RUNTIME_MAX_CONNECTIONS`

### Isolate Pool Controls

- `--pool-enabled`
  - Enable isolate pooling controls for this process.
  - Default: `false`
  - Env: `EDGE_RUNTIME_POOL_ENABLED`
- `--pool-global-max-isolates <COUNT>`
  - Global isolate cap across all functions in this process.
  - Default: `256`
  - Env: `EDGE_RUNTIME_POOL_GLOBAL_MAX_ISOLATES`
- `--pool-min-free-memory-mib <MIB>`
  - Minimum free memory required to allow isolate scale-up.
  - If memory is below this threshold, scaling is blocked and a warning is logged.
  - Default: `256`
  - Env: `EDGE_RUNTIME_POOL_MIN_FREE_MEMORY_MIB`

### Context Pool Controls (Phase 7)

- `--context-pool-enabled`
  - Enable context-first scheduling (`context -> isolate`) for routed requests.
  - Default: `false` (legacy behavior preserved).
  - Env: `EDGE_RUNTIME_CONTEXT_POOL_ENABLED`
- `--max-contexts-per-isolate <COUNT>`
  - Maximum logical contexts created per isolate before spilling into another isolate.
  - Default: `8`
  - Env: `EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE`
- `--max-active-requests-per-context <COUNT>`
  - Maximum active requests per logical context before scheduler searches another context.
  - Default: `1`
  - Env: `EDGE_RUNTIME_MAX_ACTIVE_REQUESTS_PER_CONTEXT`

### Common Options

- `--rate-limit <RATE_LIMIT>`
  - Requests per second.
  - Default: `0` (unlimited)
  - Env: `EDGE_RUNTIME_RATE_LIMIT`
- `--graceful-exit-timeout <SECONDS>`
  - Graceful shutdown deadline in seconds.
  - Default: `30`
- `--max-heap-mib <MAX_HEAP_MIB>`
  - Per-isolate heap limit in MiB.
  - Default: `128`
  - `0` means unlimited.
  - Env: `EDGE_RUNTIME_MAX_HEAP_MIB`
- `--cpu-time-limit-ms <MS>`
  - Per-request CPU limit.
  - Default: `50000`
  - `0` means unlimited.
  - Env: `EDGE_RUNTIME_CPU_TIME_LIMIT_MS`
- `--wall-clock-timeout-ms <MS>`
  - Per-request wall clock timeout.
  - Default: `60000`
  - `0` means unlimited.
  - Env: `EDGE_RUNTIME_WALL_CLOCK_TIMEOUT_MS`
- `--vfs-total-quota-bytes <BYTES>`
  - Writable VFS quota per isolate (used by `node:fs` on `/tmp`).
  - Default: `10485760` (10 MiB)
  - Env: `EDGE_RUNTIME_VFS_TOTAL_QUOTA_BYTES`
- `--vfs-max-file-bytes <BYTES>`
  - Max writable file size per isolate in VFS (`/tmp`).
  - Default: `5242880` (5 MiB)
  - Env: `EDGE_RUNTIME_VFS_MAX_FILE_BYTES`
- `--dns-doh-endpoint <URL>`
  - DNS-over-HTTPS endpoint used by `node:dns` compatibility APIs.
  - Default: `https://1.1.1.1/dns-query`
  - Env: `EDGE_RUNTIME_DNS_DOH_ENDPOINT`
- `--dns-max-answers <COUNT>`
  - Maximum number of DNS answers returned per query in `node:dns`.
  - Default: `16`
  - Env: `EDGE_RUNTIME_DNS_MAX_ANSWERS`
- `--dns-timeout-ms <MS>`
  - DNS resolver request timeout for `node:dns` DoH lookups.
  - Default: `2000`
  - Env: `EDGE_RUNTIME_DNS_TIMEOUT_MS`
- `--print-isolate-logs`
  - Print user `console.*` output from isolates into runtime logs/stdout.
  - Default: `true`
  - If disabled, isolate logs are captured only in the internal collector.
  - Env: `EDGE_RUNTIME_PRINT_ISOLATE_LOGS`
- `--sourcemap <SOURCEMAP>`
  - Source map handling for modules loaded from ESZIP.
  - Values: `none`, `inline`
  - Default: `none`
  - Env: `EDGE_RUNTIME_SOURCE_MAP`

### Authentication

When `--api-key` is set, all requests to `/_internal/*` endpoints must include the `X-API-Key` header:

```bash
# Without auth (returns 401 Unauthorized)
curl http://localhost:9000/_internal/health

# With auth (returns 200 OK)
curl -H "X-API-Key: your-secret-key" http://localhost:9000/_internal/health
```

**Security Warning:** If `--api-key` is not set, the admin API is open to all requests. A warning is logged at startup. This is acceptable for local development but **not recommended for production**.

### Internal Endpoints

All endpoints below are served on the **admin listener** (default port 9000):

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/_internal/health` | GET | Health check |
| `/_internal/metrics` | GET | Runtime and function metrics |
| `/metrics` | GET | Alias for runtime and function metrics |
| `/_internal/functions` | GET | List all deployed functions |
| `/_internal/functions` | POST | Deploy new function (body: eszip, header: `x-function-name`) |
| `/_internal/functions/{name}` | GET | Get function info |
| `/_internal/functions/{name}` | PUT | Update function |
| `/_internal/functions/{name}` | DELETE | Delete function |
| `/_internal/functions/{name}/reload` | POST | Hot reload (requires feature flag) |
| `/_internal/functions/{name}/pool` | GET | Get per-function pool limits |
| `/_internal/functions/{name}/pool` | PUT | Update per-function pool limits (`min`, `max`) |

Metrics freshness note:
- `/_internal/metrics` and `/metrics` may return cached snapshots.
- Use `?fresh=1` to force recomputation when validating just-finished load tests.

See also:
- [`docs/metrics-endpoint-reference.md`](./metrics-endpoint-reference.md) for the complete field-by-field schema reference.

#### Snapshot/V8 Compatibility

`GET /_internal/functions` and `GET /_internal/functions/{name}` include snapshot compatibility metadata per function:

- `bundle_format`: `eszip` or `snapshot`
- `package_v8_version`: V8 version recorded in the deploy package metadata
- `runtime_v8_version`: V8 version currently running in this runtime process
- `snapshot_compatible_with_runtime`: `true` when package and runtime V8 versions match
- `requires_snapshot_regeneration`: `true` when function format is `snapshot` and V8 versions mismatch
- `can_regenerate_snapshot_from_stored_eszip`: indicates whether runtime still has stored ESZIP assets to rebuild from

Operational requirement:

- If `requires_snapshot_regeneration = true`, regenerate the function snapshot using the current runtime/toolchain V8 and redeploy.
- During mismatch, runtime falls back to ESZIP startup for availability, but cold start may regress until snapshot is regenerated.

When bundle signature verification is enabled, deploy/update must include:

- Header: `x-bundle-signature-ed25519: <base64-signature>`

See: [docs/bundle-signing.md](./bundle-signing.md)

### Ingress Routing

Function requests are routed via the **ingress listener** (default port 8080):

```
GET /my-function/api/users/123
    │            └───────────────── Forwarded path: /api/users/123
    └─────────────────────────────── Function name: my-function
```

- Requests to `/_internal/*` on the ingress listener return `404 Not Found`.
- The function name is extracted from the first path segment.
- The remaining path is forwarded to the function handler.

### Examples

**Basic usage (development):**

```bash
# Start with default ports (admin: 9000, ingress: 8080)
thunder start
```

**Production with authentication:**

```bash
thunder start \
  --api-key "$(cat /run/secrets/api-key)" \
  --port 8080 \
  --max-heap-mib 256
```

**With Unix socket for ingress:**

```bash
thunder start \
  --api-key "super-secret" \
  --unix-socket /var/run/thunder.sock
```

**With TLS on both listeners:**

```bash
thunder start \
  --api-key "secret" \
  --admin-tls-cert /certs/admin.crt \
  --admin-tls-key /certs/admin.key \
  --tls-cert /certs/ingress.crt \
  --tls-key /certs/ingress.key \
  --port 8443
```

**Using environment variables:**

```bash
export EDGE_RUNTIME_API_KEY="my-secret-key"
export EDGE_RUNTIME_PORT=8080
export EDGE_RUNTIME_ADMIN_PORT=9000
thunder start
```

**Production with security hardening:**

```bash
thunder start \
  --api-key "$(cat /run/secrets/api-key)" \
  --port 8080 \
  --max-heap-mib 256 \
  --max-request-body-size 1048576 \
  --max-response-body-size 5242880 \
  --max-connections 5000
```

### Reverse Proxy Mapping (Canonical)

When using subdomain-style routing, keep the runtime canonical prefix model:

```text
External URL:  https://{function_id}.my-edge-runtime.com/...
Runtime URL:   http://localhost:8080/{function_id}/...
Admin API:     http://localhost:9000/_internal/*
```

Example:

```text
https://hello.my-edge-runtime.com/api/ping
  -> http://localhost:8080/hello/api/ping
```

Notes:

- `/{function_id}` is required on ingress requests seen by the runtime.
- Admin operations (`deploy`, `update`, `delete`) stay on the admin listener (`9000` by default), not ingress.

**Corporate network with internal service access:**

```bash
# Allow fetch() to access internal services on 10.x.x.x
thunder start \
  --api-key "secret" \
  --allow-private-net "10.0.0.0/8"
```

**Development with SSRF protection disabled:**

```bash
# Only use in development! Allows fetch() to localhost, metadata endpoints, etc.
thunder start \
  --disable-ssrf-protection
```

### Behavior Notes

- Both listeners share the same `FunctionRegistry` (deployed functions are available on both).
- Installs Rustls ring provider at startup for TLS operations.
- Initializes V8 platform on main thread before creating runtimes.
- If only one of `--tls-cert` or `--tls-key` is provided, TLS is not enabled.
- On shutdown, both listeners stop and all deployed functions are shut down.
- Unix socket file is automatically cleaned up on shutdown.
- **SSRF Protection**: Enabled by default, blocking `fetch()` to private IPs. Use `--allow-private-net` for exceptions.
- **Body Limits**: Request/response bodies exceeding limits are rejected to prevent memory exhaustion.
- **Connection Limits**: Connections exceeding the limit are dropped to prevent resource exhaustion.
- **OpenTelemetry**: when `--otel-enabled` is set, the runtime exports traces, metrics and logs via OTLP HTTP (`/v1/traces`, `/v1/metrics`, `/v1/logs`).
- **Isolate Logs to OTEL**: isolate collector export requires `--print-isolate-logs=false`; otherwise logs go to stdout and collector remains empty.

### TLS Configuration

Both listeners support HTTPS via TLS certificates. When both cert and key are provided for a listener, it serves HTTPS instead of HTTP.

#### Generating a Self-Signed Certificate (Development)

```bash
openssl req -x509 -newkey rsa:4096 \
  -keyout key.pem \
  -out cert.pem \
  -days 365 \
  -nodes \
  -subj '/CN=localhost'
```

#### Testing HTTPS

```bash
# Admin listener with self-signed certificate
curl -k -H "X-API-Key: secret" https://localhost:9000/_internal/health

# Ingress listener
curl -k https://localhost:8443/my-function/endpoint
```

#### TLS Notes

- **ALPN**: Both listeners advertise `h2` (HTTP/2) and `http/1.1` protocols via ALPN.
- **Startup log**: When TLS is enabled, the startup log shows `https://` instead of `http://`.
- **Handshake failures**: Failed TLS handshakes are logged at `warn` level.
- **Unix socket**: TLS is not supported for Unix socket ingress (redundant for local IPC).
- **Production**: For production, use certificates signed by a trusted CA.

## `bundle`

Bundles a JS/TS entrypoint and dependencies into a serialized package (ESZIP package format).

### Usage

```bash
thunder bundle --entrypoint <FILE> --output <FILE>
```

### Options

- `-e, --entrypoint <ENTRYPOINT>`
  - Required.
  - Entry JS/TS file.
- `-o, --output <OUTPUT>`
  - Required.
  - Destination file path.
- `--manifest <MANIFEST>`
  - Optional.
  - Path to a function manifest (v2).
  - When provided with `flavor: "routed-app"` and empty `routes`, the CLI auto-scans a `functions/` directory and fills `routes[]`.
  - If a sibling `public/` directory exists, static asset routes (`kind: "asset"`) are generated per file.
  - `single` manifests are not modified by this auto-scan.

### Behavior Notes

- For TS-like entrypoints (`.ts`, `.mts`, `.cts`, `.tsx`):
  - If `deno` is available in `PATH`, runs `deno check` first.
  - If not available, falls back to syntax/module-graph validation only.
- Routed-app manifest auto-scan is applied only when `--manifest` is provided and `routes[]` is empty.
- Routed-app routes are validated for build-time collisions using canonical path/method matching; ambiguous overlaps fail the bundle command.
- Routed-app bundles embed manifest JSON and route metadata (including precedence rank) into the deploy artifact.
- Result is written as a serialized `BundlePackage` containing ESZIP bytes.
- Supports `edge://assert/*` and `ext:edge_assert/*` through embedded native modules bundled in the binary (`include_str!`).

### Examples

```bash
thunder bundle \
  --entrypoint ./examples/json-api/json-api.ts \
  --output ./bundles/eszip/json-api.eszip

# With routed-app manifest auto-route generation (only when routes[] is empty)
thunder bundle \
  --entrypoint ./functions/index.ts \
  --manifest ./function.manifest.json \
  --output ./bundles/eszip/app.eszip
```

## `watch`

Watches a path for file changes, bundles discovered JS/TS files, and deploys/updates functions live.

### Usage

```bash
thunder watch [OPTIONS]
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
- `--format <eszip|snapshot>`
  - Bundle format used by watch auto-deploy pipeline.
  - Default: `snapshot`
  - `snapshot` packages a snapshot envelope with ESZIP fallback.
  - Snapshot execution may still fallback to ESZIP depending on runtime snapshot support.
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
- `--vfs-total-quota-bytes <BYTES>`
  - Writable VFS quota per isolate (`node:fs` `/tmp` mount).
  - Default: `10485760` (10 MiB)
  - Env: `EDGE_RUNTIME_VFS_TOTAL_QUOTA_BYTES`
- `--vfs-max-file-bytes <BYTES>`
  - Max writable file size per isolate in VFS.
  - Default: `5242880` (5 MiB)
  - Env: `EDGE_RUNTIME_VFS_MAX_FILE_BYTES`
- `--dns-doh-endpoint <URL>`
  - DNS-over-HTTPS endpoint used by `node:dns` compatibility APIs.
  - Default: `https://1.1.1.1/dns-query`
  - Env: `EDGE_RUNTIME_DNS_DOH_ENDPOINT`
- `--dns-max-answers <COUNT>`
  - Maximum number of DNS answers returned per query in `node:dns`.
  - Default: `16`
  - Env: `EDGE_RUNTIME_DNS_MAX_ANSWERS`
- `--dns-timeout-ms <MS>`
  - DNS resolver request timeout for `node:dns` DoH lookups.
  - Default: `2000`
  - Env: `EDGE_RUNTIME_DNS_TIMEOUT_MS`
- `--print-isolate-logs`
  - Print user `console.*` output from isolates into runtime logs/stdout.
  - Default: `true`
  - If disabled, isolate logs are captured only in the internal collector.
  - Env: `EDGE_RUNTIME_PRINT_ISOLATE_LOGS`
- `--inspect [PORT]`
  - Optional base port for V8 inspector in watch mode.
  - If provided without value, default is `9229`.
  - For multiple functions, ports auto-increment (`base`, `base+1`, ...).
- `--inspect-brk`
  - Break on first statement while waiting for debugger attach.
  - Requires `--inspect` to be useful.
- `--inspect-allow-remote`
  - Allow inspector server binding on all interfaces (`0.0.0.0`).
  - Requires `--inspect`.
  - **Security risk**: exposes debugger endpoints to the network.

### Behavior Notes

- `watch` rejects non-existing `--path`.
- Scans recursively and only considers files ending in `.ts` or `.js`.
- Skips files under directories named:
  - `node_modules`, `dist`, `build`, `.next`, `.deno`, `target`
- Converts file paths to function names by joining path segments with `-` and removing extension.
- First deployment uses `deploy`; existing function names are updated with `update`.
- Watch bundling defaults to `snapshot` package format (with ESZIP fallback at runtime).
- Server in watch mode uses immediate shutdown behavior (`graceful_exit_deadline_secs = 0`).
- **Network behavior in watch mode**:
  - SSRF protection is disabled.
  - No private/public network filtering is applied by default (all outbound network is allowed).
  - Function manifest network allowlist is not attached by watch auto-deploy (functions are deployed without manifest).
  - If a request is blocked by runtime permissions for any reason, the runtime emits a warning log with target and error.
- Inspector binds to `127.0.0.1` by default.
- Enabling inspector logs a security warning because debugger endpoints should not be exposed in production.

### Example

```bash
thunder watch \
  --path ./examples \
  --port 9000 \
  --interval 500 \
  --inspect 9230
```

## `test`

Runs JS/TS runtime test files in an isolated runtime, with optional debugger inspector support.

### Usage

```bash
thunder test [OPTIONS]
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
- `--inspect-allow-remote`
  - Allow inspector server binding on all interfaces (`0.0.0.0`).
  - Requires `--inspect`.
  - **Security risk**: exposes debugger endpoints to the network.

### Behavior Notes

- Supported test file extensions:
  - `.ts`, `.js`, `.mts`, `.mjs`
- Input discovery supports:
  - explicit file,
  - directory walk,
  - glob pattern.
- If no files are found, command exits with error.
- If `--inspect` is used, exactly one test file must be selected.
- Inspector binds to `127.0.0.1` by default.
- Enabling inspector logs a security warning because debugger endpoints should not be exposed in production.
- During execution, the command prints per-file result (`PASS`/`FAIL`), progress bar, and aggregated test stats.
- Inspector HTTP endpoints when enabled:
  - `GET /json`
  - `GET /json/list`
  - `GET /json/version`
  - WebSocket endpoint: `/ws`

### Examples

Run all tests except helper library files:

```bash
thunder test \
  --path "./tests/js/**/*.ts" \
  --ignore "./tests/js/lib/**"
```

Debug a single test file:

```bash
thunder test --path ./tests/js/my_test.ts --inspect 9229
```

## `check`

Typechecks source files with `deno check`, or falls back to syntax/module validation when Deno is unavailable.

### Usage

```bash
thunder check [OPTIONS]
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
thunder check --path ./examples
```

```bash
thunder check \
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

## WebSocket in Production

The runtime now exposes the standard `WebSocket` client API in user functions.

### Protocol and Transport

- Client WebSocket sessions use standard HTTP `Upgrade` semantics (`ws://` or `wss://`).
- For proxy hops that terminate/re-originate connections, keep upstream traffic on HTTP/1.1 for WebSocket upgrade routes.
- HTTP/2 can still be used for regular HTTP traffic in front of the proxy, but WebSocket upgrade forwarding should use HTTP/1.1 to the runtime.

### External Proxy Requirements

- Preserve upgrade headers end-to-end:
  - `Connection: Upgrade`
  - `Upgrade: websocket`
  - `Sec-WebSocket-Key`
  - `Sec-WebSocket-Version`
  - `Sec-WebSocket-Protocol` (when used)
- Disable response buffering/caching on WebSocket routes.
- Configure long-lived upstream read/write timeouts for idle-but-open sockets.
- Do not rewrite `101 Switching Protocols` responses.

### Runtime Guardrails

- Per-isolate concurrent WebSocket connection cap: `128`
- Connect timeout for sockets stuck in `CONNECTING`: `30s`

If those limits are exceeded, `WebSocket` construction fails fast with a quota-style error or the pending socket is closed on timeout.

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
