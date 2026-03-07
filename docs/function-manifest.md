# Function Manifest (v1)

This document defines the per-function manifest used by the runtime to validate deployment capabilities.

- Schema file: `schemas/function-manifest.v1.schema.json`
- Base schemas:
  - `schemas/base/common.schema.json`
  - `schemas/base/network.schema.json`
- JSON Schema draft: `2020-12`

## Goal

The manifest lets each function declare:

- Environment variables (`env.allow`, `env.secretRefs`)
- Network allowlist (`network.allow`)
- Optional resource limits (`resources`)
  - Including VFS quotas for `node:fs` compatibility in sandbox
- Optional auth and observability preferences
- Optional per-environment profile overrides (`profiles`)

API toggles like `fetch`, `crypto`, `websocket`, or `timers` are intentionally out of scope in this version.

## Security Model

The runtime enforces internal SSRF deny rules regardless of the manifest.

- A manifest cannot allow entries that collide with deny ranges from `runtime-core/src/ssrf.rs`.
- If `network.allow` contains denylisted targets, deployment fails.
- `*` is not allowed in `network.allow`.

This guarantees user manifest rules cannot bypass the hard security denylist.

At runtime, when a manifest is attached to a function:

- `network.allow` is enforced as a per-function allowlist.
- `env.allow` and `env.secretRefs` are enforced as a per-function env allowlist.
- Resource fields from `resources` are applied into `IsolateConfig` limits.
  - VFS fields from `resources` override runtime-global defaults for that function only.

When no manifest is attached, runtime behavior remains the default policy (global SSRF protection + no env access).

For debugging, when outbound requests are denied by runtime network permissions, the runtime emits a warning log including:

- request target (URL/host)
- permission error reason

## Deploy API Integration

Deploy endpoint:

- `POST /_internal/functions`
- Existing required header: `x-function-name`
- New optional header: `x-function-manifest-b64`
- New optional header: `x-function-manifest-profile`

`x-function-manifest-b64` must contain the manifest JSON encoded as Base64.

`x-function-manifest-profile`, when present, selects one profile key from `profiles` and merges it over base manifest settings.

If present, the server validates:

1. JSON decode / parse
2. JSON Schema v2020-12
3. Semantic denylist checks

If validation fails, response is `400 Bad Request` with an error payload.

## Update and Reload Behavior

- `PUT /_internal/functions/{name}`:
  - If `x-function-manifest-b64` is provided, the function manifest is replaced by the new resolved manifest.
  - If `x-function-manifest-b64` is omitted, the previously attached manifest is preserved.
- `POST /_internal/functions/{name}/reload`:
  - Keeps the currently attached manifest and reapplies it when booting the new isolate.

## Example Manifest

```json
{
  "$schema": "https://thunder.dev/schemas/function-manifest.v1.schema.json",
  "manifestVersion": 1,
  "name": "hello",
  "entrypoint": "./index.ts",
  "env": {
    "allow": ["LOG_LEVEL", "PUBLIC_BASE_URL"],
    "secretRefs": ["STRIPE_SECRET_KEY"]
  },
  "network": {
    "mode": "allowlist",
    "allow": [
      "api.example.com:443",
      "8.8.8.8",
      "8.8.8.0/24",
      "[2606:4700:4700::1111]",
      "[2606:4700:4700::1111]:443"
    ]
  },
  "resources": {
    "maxHeapMiB": 128,
    "cpuTimeMs": 50000,
    "wallClockTimeoutMs": 60000,
    "vfsTotalQuotaBytes": 10485760,
    "vfsMaxFileBytes": 5242880
  },
  "auth": {
    "verifyJwt": true
  },
  "observability": {
    "logLevel": "info",
    "traceSamplePercent": 10
  },
  "profiles": {
    "dev": {
      "network": {
        "allow": ["db.dev.local:5432"]
      }
    }
  }
}
```

## Encoding Example for Header

```bash
MANIFEST_B64="$(cat function-manifest.json | base64 | tr -d '\n')"

curl -X POST \
  -H "X-API-Key: $EDGE_RUNTIME_API_KEY" \
  -H "x-function-name: hello" \
  -H "x-function-manifest-b64: $MANIFEST_B64" \
  --data-binary @hello.eszip \
  http://127.0.0.1:9000/_internal/functions
```

## Supported `network.allow` target forms

- Host: `api.example.com`
- Host + port: `api.example.com:443`
- IPv4: `1.1.1.1`
- IPv4 + port: `1.1.1.1:443`
- IPv4 CIDR: `1.1.1.0/24`
- IPv4 CIDR + port: `1.1.1.0/24:443`
- Bracketed IPv6: `[2606:4700:4700::1111]`
- Bracketed IPv6 + port: `[2606:4700:4700::1111]:443`
- Bracketed IPv6 CIDR: `[2606:4700:4700::]/48`
- Bracketed IPv6 CIDR + port: `[2606:4700:4700::]/48:443`

## Validation Notes

Schema validation uses `jsonschema` with Draft 2020-12.
Semantic validation additionally checks denylist collisions and wildcard restrictions.

## Resource Fields for VFS

`resources` supports two VFS-specific fields:

- `vfsTotalQuotaBytes`
  - Total writable bytes allowed in `/tmp` for the isolate.
  - Default (when omitted): runtime global default (CLI/env), which defaults to `10485760` (10 MiB).
- `vfsMaxFileBytes`
  - Maximum size of a single writable file in `/tmp`.
  - Default (when omitted): runtime global default (CLI/env), which defaults to `5242880` (5 MiB).

Precedence order:

1. Per-function manifest `resources` value
2. Runtime global value from CLI flag or environment variable
3. Built-in runtime default

Validation notes:

- Values must be integers `>= 0`.
- If the runtime receives inconsistent values, effective behavior is clamped by runtime safety rules in the VFS layer.
