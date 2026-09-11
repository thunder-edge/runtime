# Thunder Edge Runtime - Current Architecture Analysis

> **Document Date:** March 7, 2026
> **Status:** Current State Assessment
> **Purpose:** Comprehensive analysis of the Thunder edge runtime platform structure to inform design decisions for filesystem-based routing and RESTful multi-method function support

---

## 1. Executive Summary

Thunder is a Rust-powered edge runtime and ecosystem built on the Deno stack, designed to run JavaScript and TypeScript functions with modern Web APIs, strong per-function isolation, and a CLI-first workflow. The current architecture uses a single-function-per-deployment model with simple name-based routing. This document analyzes the existing system structure, limitations, and deployment mechanisms to inform the transition to filesystem-based routing and RESTful multi-method function support.

---

## 2. Project Overview

### 2.1 Technology Stack

- **Language:** Rust (runtime core) + TypeScript/JavaScript (user functions)
- **JS Engine:** V8 (via Deno runtime)
- **Package Format:** ESZIP (Deno's serialized module format)
- **HTTP Server:** Hyper (async Rust HTTP framework)
- **Module System:** Deno module loader with ES6 support

### 2.2 Key Objectives

- Execute JavaScript/TypeScript in isolated V8 environments
- Provide strong per-function sandboxing with network/filesystem/resource limits
- Support dynamic function deployment via HTTP API
- Enable development workflow with hot-reload and testing
- Deliver observability (metrics, health checks, logging)

---

## 3. Current Architecture

### 3.1 High-Level Request Flow

```
┌─────────────┐
│   Client    │
└──────┬──────┘
       │ HTTP Request
       │ GET /function-name/path
       ↓
┌──────────────────────────────────────────┐
│     Thunder CLI (dev mode) OR            │
│     Thunder HTTP Server                  │
│     crates/server                        │
└──────────────┬───────────────────────────┘
               │
               ├─→ Ingress Router
               │   (Extract function name
               │    from first URL segment)
               │
               ├─→ Function Registry
               │   (Lookup isolate handle)
               │
               ├─→ Isolate Manager
               │   (Route to V8 isolate)
               │
               └─→ User Function Handler
                   (Deno.serve() pattern)
                   └─→ Response
```

### 3.2 Repository Structure

```
thunder/
├── crates/
│   ├── cli/               # CLI binary (bundle, start, watch, test, check)
│   ├── server/            # HTTP server & routing logic
│   ├── functions/         # Function lifecycle & registry
│   └── runtime-core/      # V8 + Deno extensions + sandbox
│
├── docs/                  # Guides and API references
├── examples/              # Example functions (hello, json-api, etc.)
├── schemas/               # JSON Schema for manifests
├── scripts/               # Build & deployment automation
└── tests/                 # Integration tests

```

### 3.3 Core Crates

#### 3.3.1 `crates/runtime-core`

**Purpose:** V8 runtime environment with Deno extensions

**Key Components:**
- `isolate.rs` - V8 isolate creation, configuration, request handling
- `manifest.rs` - Function manifest parsing and validation
- `extensions.rs` - Deno Web APIs (fetch, crypto, streams, etc.)
- `permissions.rs` - Network/filesystem/environment permission enforcement
- `module_loader.rs` - ESZIP module loading and resolution
- `node_compat/` - Node.js compatibility layer
- `assert/` - Built-in test assertion library

**Key Exports:**
```rust
pub struct JsRuntime { ... }  // V8 runtime instance
pub struct IsolateConfig { ... }  // Isolate configuration
pub struct FunctionManifest { ... }  // Manifest structure
```

#### 3.3.2 `crates/functions`

**Purpose:** Function lifecycle management and routing registry

**Key Components:**
- `registry.rs` - Thread-safe registry of deployed functions
- `handler.rs` - Request routing to isolate handlers
- `lifecycle.rs` - Function deploy/update/delete operations
- `metrics.rs` - Per-function metrics collection
- `types.rs` - Type definitions for function entries

**Responsibilities:**
- Deploy/update/delete functions from the registry
- Maintain isolate pools per function
- Route incoming requests to correct isolate
- Collect per-function metrics

#### 3.3.3 `crates/server`

**Purpose:** HTTP server and ingress/admin routing

**Key Components:**
- `ingress_router.rs` - Routes `/{function_name}/*` requests
- `admin_router.rs` - Handles `/_internal/*` management endpoints
- `router.rs` - Shared routing utilities
- `body_limits.rs` - Request/response size limit enforcement
- `bundle_signature.rs` - Optional Ed25519 bundle signature verification

**Routing Logic:**
```rust
// ingress_router.rs (simplified)
let path = req.uri().path();
let segments: Vec<&str> = path.splitn(3, '/').collect();
// segments: ["", "function_name", "rest/of/path"]
let function_name = segments[1];  // ONLY uses first segment
let forwarded_path = format!("/{}", segments[2]);  // Pass rest to handler
```

#### 3.3.4 `crates/cli`

**Purpose:** Command-line tools for development and operations

**Commands:**
- `start` - Starts the HTTP server for function invocation
- `bundle` - Generates ESZIP package from entrypoint
- `watch` - Watches for file changes and hot-reloads
- `test` - Runs JavaScript/TypeScript tests inside the runtime
- `check` - Runs typecheck/syntax validation

---

## 4. Current Function Format

### 4.1 Supported Pattern: Deno.serve() Handler

**Current ONLY supported pattern:**

```typescript
// File: functions/index.ts (or any entrypoint)
import { serve } from "https://deno.land/std@x.y.z/http/server.ts";

const handler = (req: Request): Response => {
  // User implements all routing manually
  const url = new URL(req.url);

  if (url.pathname === "/api/users" && req.method === "GET") {
    return new Response(JSON.stringify([/* users */]), {
      headers: { "content-type": "application/json" }
    });
  }

  if (url.pathname.startsWith("/api/users/") && req.method === "GET") {
    const id = url.pathname.split("/").pop();
    return new Response(JSON.stringify({ id }), {
      headers: { "content-type": "application/json" }
    });
  }

  return new Response("Not Found", { status: 404 });
};

serve(handler);  // Must explicitly call serve()
```

### 4.2 Example: JSON API Handler

**Current pattern with manual routing:**

```typescript
// examples/json-api/json-api.ts

const routes = {
  "/api/users": {
    GET: () => [
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
    ],
  },
  "/api/users/:id": {
    GET: (id: string) => ({ id: parseInt(id), name: `User ${id}` }),
  },
  "/api/echo": {
    POST: async (body: unknown) => ({ message: "Echo received", data: body }),
  },
};

Deno.serve(async (req) => {
  const url = new URL(req.url);
  const method = req.method;
  const pathname = url.pathname;

  if (method === "OPTIONS") {
    return new Response(null, {
      headers: {
        "access-control-allow-origin": "*",
        "access-control-allow-methods": "GET, POST, OPTIONS",
      },
    });
  }

  // Explicit route matching for each endpoint
  if (pathname === "/api/users" && method === "GET") {
    const users = routes["/api/users"].GET();
    return new Response(JSON.stringify(users), {
      headers: { "content-type": "application/json" }
    });
  }

  const userMatch = pathname.match(/^\/api\/users\/(\d+)$/);
  if (userMatch && method === "GET") {
    const user = routes["/api/users/:id"].GET(userMatch[1]);
    return new Response(JSON.stringify(user), {
      headers: { "content-type": "application/json" }
    });
  }

  // ... more explicit route matching ...

  return new Response(JSON.stringify({ error: "Not found" }), { status: 404 });
});
```

### 4.3 Current Pattern Limitations

1. **Verbose Routing:** Users must implement all routing logic inside the handler using string parsing or URLPattern
2. **No IDE Support:** Routing logic is opaque to editors; no type checking for route parameters
3. **Manual HTTP Method Dispatch:** No framework-like method-specific handlers
4. **No Metadata:** Routes are implicit in code; runtime has no awareness of route structure
5. **Code Organization:** Directory structure doesn't reflect API structure; all routes in one function

---

## 5. Deployment Model

### 5.1 Current Deployment Process

**Step 1: Bundle Creation**

```bash
cargo run -- bundle \
  --entrypoint ./examples/hello/hello.ts \
  --output ./hello.eszip
```

**What happens:**
1. CLI loads the entrypoint file
2. Deno's module graph builder resolves all dependencies
3. Files are bundled into ESZIP format (serialized ES modules)
4. Output: Binary ESZIP file containing all code + dependencies

**Key constraint:** Single entrypoint per bundle

**Step 2: Deploy to Runtime**

```bash
curl -X POST http://localhost:9000/_internal/functions \
  -H "x-function-name: hello" \
  -H "x-function-manifest-b64: $(base64 < manifest.json)" \
  --data-binary @./hello.eszip
```

**What happens:**
1. Server receives POST request with binary ESZIP body
2. Function name extracted from `x-function-name` header
3. Optional manifest extracted from `x-function-manifest-b64` header (base64-encoded JSON)
4. Function registry creates new isolate and stores bundle
5. Function becomes available at `GET /hello/*`

**Step 3: Invocation**

```bash
curl http://localhost:9000/hello
curl http://localhost:9000/hello/subpath
```

**What happens:**
1. Ingress router extracts function name from first URL segment
2. Looks up isolate handle in registry
3. Rewrites path to remove function name prefix (e.g., `/hello/subpath` → `/subpath`)
4. Sends request to isolate's handler
5. Returns response to client

### 5.2 Admin API Endpoints

```
POST   /_internal/functions               # Deploy new function
GET    /_internal/functions               # List deployed functions
GET    /_internal/functions/{name}        # Get function info
PUT    /_internal/functions/{name}        # Update function
DELETE /_internal/functions/{name}        # Delete function
POST   /_internal/functions/{name}/reload # Hot-reload (feature-gated)
GET    /_internal/health                  # Health check
GET    /_internal/metrics                 # Prometheus metrics
```

---

## 6. Function Manifest

### 6.1 Current Manifest Schema

**Schema Location:** `/schemas/function-manifest.v1.schema.json`

**Current Manifest Structure:**

```json
{
  "manifestVersion": 1,
  "name": "my-function",
  "entrypoint": "index.ts",
  "env": {
    "allow": ["DATABASE_URL", "API_KEY"],
    "secretRefs": ["JWT_SECRET"]
  },
  "network": {
    "mode": "allowlist",
    "allow": [
      "api.example.com:443",
      "db.example.com:5432"
    ]
  },
  "resources": {
    "maxHeapMiB": 256,
    "cpuTimeMs": 5000,
    "wallClockTimeoutMs": 30000,
    "vfsTotalQuotaBytes": 10485760,
    "vfsMaxFileBytes": 1048576
  },
  "auth": {
    "verifyJwt": true
  },
  "observability": {
    "logLevel": "info",
    "traceSamplePercent": 10
  },
  "profiles": {
    "staging": {
      "env": { "allow": ["STAGING_DB_URL"] },
      "network": { "allow": ["staging-api.example.com"] },
      "resources": { "maxHeapMiB": 512 },
      "observability": { "traceSamplePercent": 50 }
    },
    "production": {
      "env": { "allow": ["PROD_DB_URL"] },
      "resources": { "maxHeapMiB": 1024, "cpuTimeMs": 10000 }
    }
  }
}
```

### 6.2 Manifest Properties

#### Top-Level Properties

| Property | Type | Required | Purpose |
|----------|------|----------|---------|
| `manifestVersion` | integer | Yes | Schema version (currently 1) |
| `name` | string | Yes | Function identifier (alphanumeric + hyphens) |
| `entrypoint` | string | Yes | Entry module path (e.g., "index.ts") |
| `env` | object | No | Environment variable access control |
| `network` | object | Yes | Network access policy (allowlist-only) |
| `resources` | object | No | Memory, CPU, timeout, filesystem limits |
| `auth` | object | No | Authentication/authorization settings |
| `observability` | object | No | Logging and tracing configuration |
| `profiles` | object | No | Environment-specific overrides |

#### Environment Configuration

```json
{
  "env": {
    "allow": ["VAR1", "VAR2"],           // Allowed env vars
    "secretRefs": ["SECRET1", "SECRET2"]  // Reference to secret manager
  }
}
```

#### Network Configuration

```json
{
  "network": {
    "mode": "allowlist",  // Only mode supported
    "allow": [
      "example.com",                     // Domain
      "api.example.com:443",             // Domain + port
      "10.0.0.0/8",                      // CIDR block
      "192.168.1.1",                     // IP address
      "2001:db8::/32"                    // IPv6 CIDR
    ]
  }
}
```

#### Resource Limits

```json
{
  "resources": {
    "maxHeapMiB": 256,              // V8 heap size limit
    "cpuTimeMs": 5000,              // CPU time limit (async-aware)
    "wallClockTimeoutMs": 30000,    // Total request timeout
    "vfsTotalQuotaBytes": 10485760, // Total filesystem quota
    "vfsMaxFileBytes": 1048576      // Max file size
  }
}
```

#### Profiles (Environment-Specific)

Profiles allow environment-specific overrides:

```json
{
  "profiles": {
    "staging": {
      "resources": { "maxHeapMiB": 512 },
      "observability": { "traceSamplePercent": 50 }
    },
    "production": {
      "resources": { "maxHeapMiB": 1024 }
    }
  }
}
```

### 6.3 Manifest Validation

**Key Constraints:**
- Must conform to JSON Schema Draft 2020-12
- Network mode must be "allowlist" (deny-by-default)
- Function name must match pattern: `^[a-z][a-z0-9_-]{0,31}$`
- Manifest must be valid UTF-8 JSON when base64-encoded in headers

**Validation Location:** `crates/runtime-core/src/manifest.rs`

```rust
pub struct FunctionManifest {
    pub manifest_version: u32,
    pub name: String,
    pub entrypoint: String,
    pub env: Option<ManifestEnv>,
    pub network: ManifestNetwork,
    pub resources: Option<ManifestResources>,
    pub auth: Option<ManifestAuth>,
    pub observability: Option<ManifestObservability>,
    pub profiles: HashMap<String, ManifestProfile>,
}
```

---

## 7. Current Routing Logic

### 7.1 Ingress Routing Implementation

**Location:** `crates/server/src/ingress_router.rs`

**Algorithm (Simplified):**

```rust
async fn route_to_function(&self, req: Request, trace_ctx: &TraceContext) {
    let path = req.uri().path().to_string();
    let method = req.method().clone();

    // Extract function name from FIRST path segment ONLY
    let segments: Vec<&str> = path.splitn(3, '/').collect();
    // For path="/hello/world", segments=["", "hello", "world"]
    let function_name = segments.get(1).unwrap_or(&"");

    // Validation
    if function_name.is_empty() {
        return 404_response("no function specified");
    }

    // Lookup isolate
    let Some(handle) = self.registry.get_handle(function_name) else {
        return 404_response("function not found");
    };

    // Rewrite path: strip function_name prefix
    let forwarded_path = if segments.len() >= 3 {
        format!("/{}", segments[2])
    } else {
        "/".to_string()
    };

    // Create new request with rewritten path
    let forwarded_req = Request::builder()
        .method(method)
        .uri(&forwarded_path)
        .body(body_bytes)
        .unwrap();

    // Send to isolate
    let response = handle.send_request(forwarded_req).await?;

    Ok(response)
}
```

### 7.2 Routing Constraints

**Current Limitations:**

1. **Single Segment Extraction:** Only the first URL segment determines function identity
   ```
   GET /hello/world          → function "hello", forwarded as GET /world
   GET /api/v1/users/123     → function "api", forwarded as GET /v1/users/123
   GET /users                → function "users", forwarded as GET /
   ```

2. **No Pattern Matching:** No support for parameterized paths
   ```
   // NOT SUPPORTED:
   GET /users/:id            (id parameter extraction)
   GET /posts/:slug          (slug parameter extraction)
   GET /admin/*              (catch-all routes)
   ```

3. **No HTTP Method Routing:** Method available but not used for routing decisions
   ```
   // Can't do:
   POST /users               (different from GET /users)
   DELETE /users/:id         (specific method handling)
   ```

4. **No Route Metadata:** Runtime has no awareness of available routes
   ```
   // Cannot:
   - List available routes
   - Optimize hot paths
   - Detect conflicts
   - Provide per-route metrics
   ```

5. **No Route Validation:** Route structure determined entirely by user code
   ```
   // Possible issues:
   - Route conflicts undetected at deploy time
   - Manual parameter parsing error-prone
   - No type safety for parameters
   ```

### 7.3 Ingress Request Flow

```
Incoming Request: GET /hello/world/subpath

1. parse path:
   path = "/hello/world/subpath"

2. extract segments:
   segments = ["", "hello", "world/subpath"]

3. lookup function:
   function_name = "hello"
   handle = registry.get("hello")
   ✓ Found

4. rewrite path:
   forwarded_path = "/world/subpath"

5. forward to isolate:
   isolate.send_request(
     method: GET
     uri: /world/subpath
     body: ...
     headers: ...
   )

6. isolate handler processes request:
   Deno.serve(async (req) => {
     // req.url contains /world/subpath
     // User implements routing manually
   })

7. return response to client
```

---

## 8. Current Limitations

### 8.1 Developer Experience Issues

| Issue | Current Behavior | Impact |
|-------|------------------|--------|
| **Manual Routing** | Users implement URL pattern matching in code | Verbose boilerplate, repetitive logic |
| **No IDE Support** | Routes are opaque to editors | No autocomplete, hard to refactor |
| **Parameter Parsing** | Manual extraction using regex/string parsing | Error-prone, no type checking |
| **No Framework Alignment** | Incompatible with Next.js, Remix, SvelteKit | Developers expect filesystem routing |
| **Code Organization** | One function = one directory = monolithic bundle | Doesn't scale for APIs with many routes |

### 8.2 Operational Limitations

| Issue | Current Behavior | Impact |
|-------|------------------|--------|
| **No Per-Route Metrics** | Metrics aggregated at function level | Hard to identify slow/failed routes |
| **No Route Metadata** | Runtime unaware of route structure | Impossible to validate at deploy time |
| **Single Entrypoint** | One ESZIP per bundle | Can't deploy multiple related functions together |
| **Route Collisions** | Not detected or reported | Silent behavior changes on updates |
| **Limited Introspection** | No way to query available routes | Makes automation/monitoring harder |

### 8.3 Technical Limitations

| Issue | Current Behavior | Impact |
|-------|------------------|--------|
| **String-Based Routing** | Manual URL parsing | Performance overhead, no optimization |
| **Handler Interception** | Global `Deno.serve()` replacement | Couples user code to runtime |
| **Manifest Gaps** | No route definitions in manifest | Route information lost at deploy time |
| **No HTTP Method Dispatch** | All methods go to same handler | Can't enforce method-specific behavior |
| **Implicit Routes** | Routes defined in code only | Hard to detect misconfiguration |

---

## 9. Current System Diagram

### 9.1 Component Interaction Diagram

```
┌────────────────────────────────────────────────────────────────────┐
│                        HTTP Client / CLI                           │
└────────────────────────┬───────────────────────────────────────────┘
                         │
                         ├─── Development Flow (cargo run -- watch)
                         │    └─→ Auto-bundle & hot-reload
                         │
                         └─── Production Flow (HTTP API)
                              ├─→ POST /_internal/functions (deploy)
                              └─→ GET /function-name/* (invoke)

                ┌─────────────────────────────────────────────┐
                │    Thunder HTTP Server                      │
                │    crates/server                            │
                ├─────────────────────────────────────────────┤
                │                                             │
                │  ┌────────────────────────────────────────┐ │
                │  │  Admin Router                          │ │
                │  │  /_internal/*                          │ │
                │  ├────────────────────────────────────────┤ │
                │  │ POST /_internal/functions              │ │
                │  │ PUT /_internal/functions/{name}        │ │
                │  │ DELETE /_internal/functions/{name}     │ │
                │  └────────────────────────────────────────┘ │
                │                                             │
                │  ┌────────────────────────────────────────┐ │
                │  │  Ingress Router                        │ │
                │  │  /{function_name}/*                    │ │
                │  ├────────────────────────────────────────┤ │
                │  │ 1. Extract function_name from         │ │
                │  │    first URL segment                  │ │
                │  │ 2. Lookup isolate in registry         │ │
                │  │ 3. Rewrite path (strip function name) │ │
                │  │ 4. Forward request to isolate         │ │
                │  └────────────────────────────────────────┘ │
                │                                             │
                └──────────────────┬──────────────────────────┘
                                   │
                ┌──────────────────▼──────────────────┐
                │   Function Registry                  │
                │   crates/functions                   │
                ├──────────────────────────────────────┤
                │ - Deploy/update/delete functions     │
                │ - Maintain isolate handles           │
                │ - Route requests to correct isolate  │
                │ - Collect per-function metrics       │
                └──────────────────┬──────────────────┘
                                   │
                ┌──────────────────▼──────────────────┐
                │   Isolate Pool                       │
                │   (per function)                     │
                ├──────────────────────────────────────┤
                │                                      │
                │  ┌──────────────────────────────┐   │
                │  │  Isolate #1 (Primary)        │   │
                │  │  ┌────────────────────────┐  │   │
                │  │  │  V8 Runtime            │  │   │
                │  │  │  (crates/runtime-core) │  │   │
                │  │  │                        │  │   │
                │  │  │  User Handler:         │  │   │
                │  │  │  Deno.serve(() => {}) │  │   │
                │  │  └────────────────────────┘  │   │
                │  │  - Memory limit: 256 MiB     │   │
                │  │  - CPU timeout: 5s           │   │
                │  │  - Permissions: allowlist    │   │
                │  └──────────────────────────────┘   │
                │                                      │
                │  ┌──────────────────────────────┐   │
                │  │  Isolate #2 (Spare)          │   │
                │  │  ... (created on demand)     │   │
                │  └──────────────────────────────┘   │
                │                                      │
                └──────────────────────────────────────┘

User Function Bundle:
┌────────────────────────────────┐
│  ESZIP Package                 │
├────────────────────────────────┤
│  index.ts (entrypoint)         │
│  dependencies.ts               │
│  ... other modules ...         │
│  Thunder Manifest (embedded)   │
└────────────────────────────────┘
```

### 9.2 Deployment Sequence Diagram

```
Client                    Server                Registry            Isolate
  │                         │                      │                  │
  │ POST /functions         │                      │                  │
  ├────────────────────────>│                      │                  │
  │ (ESZIP + manifest)      │                      │                  │
  │                         │                      │                  │
  │                         │ validate manifest   │                  │
  │                         │ parse ESZIP         │                  │
  │                         │                      │                  │
  │                         │ deploy(name, bundle)│                  │
  │                         ├─────────────────────>│                  │
  │                         │                      │ create isolate   │
  │                         │                      ├─────────────────>│
  │                         │                      │ load ESZIP       │
  │                         │                      │<─────────────────┤
  │                         │                      │ ready            │
  │                         │                      │                  │
  │                         │ store(handle)       │                  │
  │                         │<─────────────────────┤                  │
  │ 201 Created             │                      │                  │
  │<────────────────────────┤                      │                  │
  │                         │                      │                  │
  │                         │                      │                  │
  │ GET /function-name/path │                      │                  │
  ├────────────────────────>│                      │                  │
  │                         │ lookup(function-name)│                  │
  │                         ├─────────────────────>│                  │
  │                         │<─────────────────────┤ handle           │
  │                         │ rewrite path        │                  │
  │                         │ forward request     │                  │
  │                         ├──────────────────────────────────────>│
  │                         │                      │ Deno.serve()    │
  │                         │                      │ processes req   │
  │                         │                      │ (manual routing)│
  │                         │<──────────────────────────────────────┤
  │ 200 OK                  │ return response     │                  │
  │<────────────────────────┤                      │                  │
```

---

## 10. Development Workflow

### 10.1 Local Development with Watch Mode

```bash
cargo run -- watch --path ./examples --port 9000 --inspect 9229
```

**What happens:**
1. CLI watches for `.ts` and `.js` file changes
2. On file change:
   - Triggers bundle generation
   - Deploys to running server
   - Updates function registry
3. Isolates hot-reload (if feature enabled)
4. V8 inspector accessible on port 9229 for debugging

### 10.2 Testing in Runtime

```bash
cargo run -- test --path "./tests/js/**/*.ts" --ignore "./tests/js/lib/**"
```

**What happens:**
1. CLI finds test files matching pattern
2. Each test file executed in isolated V8 runtime
3. Test assertions via `edge://assert/*` library
4. Results reported to CLI

### 10.3 Type Checking

```bash
cargo run -- check --path "./**/*.{ts,js,mts,mjs,tsx,jsx,cjs,cts}"
```

**What happens:**
1. If Deno available: delegates to `deno check`
2. Otherwise: runs syntax validation only
3. Reports TypeScript errors to CLI

---

## 11. Observability

### 11.1 Metrics Endpoint

```
GET /_internal/metrics
```

Returns Prometheus-format metrics:

```
# Per-function metrics
thunder_function_requests_total{function="hello"} 42
thunder_function_errors_total{function="hello"} 2
thunder_function_duration_seconds_bucket{function="hello",le="0.1"} 38
thunder_function_duration_seconds_bucket{function="hello",le="0.5"} 41
thunder_function_duration_seconds_bucket{function="hello",le="1"} 42

# Runtime metrics
thunder_isolate_count{function="hello"} 1
thunder_isolate_memory_bytes{function="hello"} 67108864
```

### 11.2 Logging

- Structured logging via `tracing` crate
- Per-request trace ID for correlation
- Log level configurable per function (manifest)
- OpenTelemetry (OTEL) integration available

### 11.3 Health Check

```
GET /_internal/health
```

Returns:
```json
{
  "status": "ok",
  "version": "0.1.0",
  "timestamp": "2024-03-07T12:34:56Z"
}
```

---

## 12. Security Model

### 12.1 Function Isolation

**Per-isolate:**
- Separate V8 heap (independent garbage collection)
- Independent module namespaces
- Isolated globals (no cross-function data sharing)
- Separate request queues

**Per-sandbox:**
- Network access: allowlist only (SSRF protection)
- Filesystem: read-only VFS with quota limits
- Environment variables: explicit allowlist
- No subprocess execution
- No native code execution (except extensions)

### 12.2 Manifest-Based Permissions

```json
{
  "network": {
    "mode": "allowlist",
    "allow": ["api.example.com:443"]  // Only HTTPS to this domain
  },
  "env": {
    "allow": ["DATABASE_URL"],        // Only this env var
    "secretRefs": ["API_KEY"]          // Only this secret
  },
  "resources": {
    "maxHeapMiB": 256,                // Memory limit
    "cpuTimeMs": 5000,                // CPU time limit
    "wallClockTimeoutMs": 30000       // Request timeout
  }
}
```

### 12.3 Current Security Issues (from AUDIT.md)

1. **Unauthenticated Admin Endpoints:** `/_internal/*` endpoints not yet authenticated
2. **TLS Not Applied:** TLS configured but not enforced in accept loop
3. **SSRF Risk:** Network rules permissive (mitigated by allowlist)
4. **No Body Size Limits:** Request/response body limits not enforced (recently added)

---

## 13. What Needs to Change: Roadmap Context

### 13.1 Filesystem-Based Routing

**Target Structure:**
```
functions/
  index.ts                    # Route: /
  about.ts                    # Route: /about
  api/
    users.ts                  # Route: /api/users
    users/
      [id].ts                 # Route: /api/users/:id
      [id]/
        posts.ts              # Route: /api/users/:id/posts
    posts/
      [slug].ts               # Route: /api/posts/:slug
  admin/
    [...admin].ts             # Route: /admin/*
  blog/
    [[lang]].ts               # Route: /blog or /blog/:lang
```

**Impact on Current System:**
- Bundle creation must scan directory tree
- Generate route manifest at build time
- Store routes in manifest for runtime discovery
- Modified ingress router to match routes before forwarding

### 13.2 RESTful Multi-Method Support

**Target Patterns:**

```typescript
// Pattern 1: Default Function Export
export default async function handler(
  req: Request,
  params?: Record<string, string>
): Promise<Response> {
  return new Response("Hello");
}

// Pattern 2: HTTP Method Object
export default {
  async GET(req, params) { return new Response("GET"); },
  async POST(req, params) { return new Response("POST", { status: 201 }); },
  async DELETE(req, params) { return new Response("", { status: 204 }); }
};

// Pattern 3: Named HTTP Exports
export async function GET(req, params) { return new Response("GET"); }
export async function POST(req, params) { return new Response("POST", { status: 201 }); }
```

**Impact on Current System:**
- Runtime must detect export pattern (Deno.serve vs. exports)
- Create adapter layer for new patterns
- Maintain backward compatibility with Deno.serve()
- Route-specific method dispatch at ingress level

### 13.3 New Manifest Format

**Required Additions:**
```json
{
  "manifestVersion": 2,  // Incremented
  "routes": [
    {
      "path": "/api/users",
      "pattern": "/api/users",
      "entrypoint": "api/users.ts",
      "methods": ["GET", "POST"]
    },
    {
      "path": "/api/users/:id",
      "pattern": "/api/users/[id]",
      "entrypoint": "api/users/[id].ts",
      "methods": ["GET", "PUT", "DELETE"],
      "parameters": [{ "name": "id", "type": "string" }]
    }
  ],
  // ... existing fields ...
}
```

**Impact:**
- Manifest validation updated to v2
- Route priority calculated at deploy time
- Per-route resource limits (future enhancement)

---

## 14. Key Takeaways for Implementation

### 14.1 System Design Principles

1. **Backward Compatibility:** Existing `Deno.serve()` functions continue to work
2. **Single Bundle, Multiple Routes:** One deployment contains many related functions
3. **Build-Time Generation:** Route manifest generated during bundling
4. **Declarative Routing:** Routes defined by filesystem structure, not code
5. **Type Safety:** Parameters and handlers are type-aware

### 14.2 Critical Components to Modify

| Component | Current | Change Required |
|-----------|---------|-----------------|
| **Bundle CLI** | Single entrypoint | Scan directory, generate route manifest |
| **Ingress Router** | Name-based only | Pattern matching with parameter extraction |
| **Function Registry** | Single handler per function | Multiple routes per function |
| **Manifest Schema** | No routes section | Add `routes` array with path patterns |
| **Isolate Handler** | Deno.serve() only | Support export-based patterns with adapter |
| **Admin API** | Deploy single function | Deploy multiple routes in one bundle |

### 14.3 Backward Compatibility Strategy

```
Detection at isolate startup:
  ├─ If Deno.serve() called in code
  │  └─ Use existing behavior (works with any path)
  │
  └─ If exports found (default/methods/named)
     └─ Use new handler adapter
        └─ Route to correct handler based on method
        └─ Extract parameters from path
        └─ Call handler with (req, params)
```

### 14.4 Testing Strategy

1. **Unit Tests:** Route pattern matching logic
2. **Integration Tests:** Full deployment + invocation flow
3. **Compatibility Tests:** Existing Deno.serve() functions still work
4. **Performance Tests:** Route matching overhead vs. monolithic handlers

---

## 15. File Reference Guide

### Core Architecture Files

| Path | Purpose |
|------|---------|
| `crates/server/src/ingress_router.rs` | Request routing logic (needs modification) |
| `crates/server/src/admin_router.rs` | Admin API endpoints (needs deployment API updates) |
| `crates/functions/src/registry.rs` | Function registry (needs multi-route support) |
| `crates/functions/src/handler.rs` | Request handling (needs adapter for new patterns) |
| `crates/runtime-core/src/manifest.rs` | Manifest structs (needs v2 schema) |
| `crates/runtime-core/src/isolate.rs` | V8 isolate (needs handler pattern detection) |
| `crates/cli/src/commands/bundle.rs` | Bundle creation (needs directory scanning) |
| `schemas/function-manifest.v1.schema.json` | Manifest schema (needs v2 version) |

### Configuration & Documentation

| Path | Purpose |
|------|---------|
| `README.md` | Project overview |
| `docs/reference/cli.md` | CLI command reference |
| `AUDIT.md` | Security audit findings |

### Examples

| Path | Purpose |
|------|---------|
| `examples/hello/hello.ts` | Simplest Deno.serve() function |
| `examples/json-api/json-api.ts` | Complex routing with manual implementation |
| `examples/error-handling/error-handling.ts` | Error handling patterns |

---

## 16. Glossary

| Term | Definition |
|------|-----------|
| **Isolate** | A V8 JavaScript runtime instance with its own heap and global scope |
| **ESZIP** | Deno's serialized module format; bundles code and dependencies into a single binary |
| **Function Manifest** | JSON configuration file defining function behavior (permissions, limits, entrypoint) |
| **Ingress** | The public-facing HTTP router that accepts user requests |
| **Admin Router** | The internal management API under `/_internal/*` |
| **Registry** | In-memory map of deployed functions to their isolate handles |
| **Handler** | User-defined function that processes HTTP requests |
| **Handler Adapter** | Runtime layer that converts different export patterns to a standard interface |

---

## 17. Conclusion

Thunder is a sophisticated edge runtime with strong isolation guarantees and flexible deployment. The current single-function-per-deployment model with name-based routing works well for simple use cases but becomes cumbersome for complex APIs. The proposed transition to filesystem-based routing and RESTful multi-method support will modernize the developer experience while maintaining the runtime's core strengths in isolation and observability.

The key to successful implementation lies in:
1. Preserving backward compatibility with existing Deno.serve() functions
2. Building route metadata at build time (bundling phase)
3. Implementing efficient pattern matching in the ingress router
4. Supporting multiple export patterns with a unified adapter layer
5. Maintaining per-route isolation and observability

This analysis provides the foundation for understanding what needs to change and where the changes should be implemented.
