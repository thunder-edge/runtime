# ROADMAP_ROUTING

> Status: Engineering Design and Execution Roadmap
>
> Date: March 7, 2026
>
> Scope: Filesystem-based routing, multi-function deployment flavor, global domain routing manifest, and a RESTful function contract based on `export default`
>
> Related documents: `ROADMAP.md`, `ROADMAP_OPENNEXT.md`, `docs/reference/function-manifest.md`, `docs/function-contract-design.md`, `docs/reference/cli.md`, `CURRENT_ARCHITECTURE_ANALYSIS.md`, `schemas/function-manifest.v2.schema.json`

---

## Table of Contents

1. [Objective](#objective)
2. [Current Platform Baseline](#current-platform-baseline)
3. [Problem Statement](#problem-statement)
4. [Vision & Success Metrics](#vision--success-metrics)
5. [Proposed Architecture](#proposed-architecture)
6. [Function Flavor Support](#function-flavor-support)
7. [Detailed Implementation Roadmap](#detailed-implementation-roadmap)
8. [File-by-File Implementation Details](#file-by-file-implementation-details)
9. [Real-World Examples](#real-world-examples)
10. [Phase-by-Phase Breakdown](#phase-by-phase-breakdown)
11. [Migration Strategy](#migration-strategy)
12. [Performance Impact Analysis](#performance-impact-analysis)
13. [Security & Edge Cases](#security--edge-cases)
14. [Testing Strategy](#testing-strategy)
15. [Open Questions & Design Decisions](#open-questions--design-decisions)
16. [Related ROADMAP.md Tasks](#related-roadmapmd-tasks)

---

## 1. Objective

This document defines the target architecture to evolve Thunder from the current model:

- Ingress routing by `/{function_name}/*`
- One deployed function per bundle
- Single manifest entrypoint
- User code centered on `Deno.serve()` interception

To a model that supports:

- **Filesystem-based routing** inspired by Next.js, Remix, SvelteKit, and Astro
- **Two deploy flavors:**
  - `single`: One self-contained file that handles all requests
  - `routed-app`: One deployment containing multiple routed modules, suitable for REST APIs or frontend apps
- **Preserved deployment mount prefix** so the runtime keeps serving requests under `/{function_id}/...`
- **RESTful authoring contract** based on `export default`
- **Runtime method dispatch** with `405 Method Not Allowed` and `Allow` headers
- **Route metadata** available at build time, deploy time, and runtime

The goal is to give the platform a framework-friendly substrate that is minimal, predictable, and compatible with edge-runtime constraints.

---

## 2. Current Platform Baseline

### 2.1 Request and Routing Model Today

The current ingress path resolution is implemented in `crates/server/src/ingress_router.rs`.

**Current behavior:**
- The first path segment is interpreted as the function name
- The remaining suffix is forwarded to the isolate unchanged
- No filesystem discovery exists
- No route manifest exists
- No path parameter extraction exists
- HTTP method is not used for routing before reaching user code

**Current request flow:**
```
Request: GET /my-function/api/users/123
  ↓
ingress_router extracts "my-function"
  ↓
registry resolves one deployed function
  ↓
path forwarded as "/api/users/123"
  ↓
user handler parses URL and method manually
  ↓
Response
```

This gives maximum flexibility but pushes all routing complexity into each user function.

### 2.1.1 Addressing Model Used by the Runtime Today

Today the runtime canonical addressing model is path-prefix based:

```text
http://localhost:9000/function_id_xpto/api/xpto
```

That first segment is not an implementation detail to be removed. It is the deployment identifier used by the runtime to decide which function bundle or routed app should receive the request.

This constraint matters operationally because the intended edge topology is:

```text
https://function_id_xpto.my-edge-runtime.com/api/xpto
  -> reverse proxy rewrite
http://localhost:9000/function_id_xpto/api/xpto
```

Therefore, any routing redesign must preserve these invariants:

- The runtime continues to accept `/{function_id}/...` as the canonical ingress format
- Filesystem routing happens only inside the namespace of one deployed function/app
- Reverse proxies may map hostnames to path prefixes, but the runtime contract remains path-prefix based
- The deploy identifier remains visible and stable for local debugging, internal traffic, and non-proxied environments

### 2.2 Deploy Model Today

The current admin API is centered on `POST /_internal/functions` with:
- `x-function-name` header
- Optional `x-function-manifest-b64` header (base64-encoded JSON)
- Request body containing one ESZIP bundle

**Current manifest v1** (from `schemas/function-manifest.v1.schema.json`):
```json
{
  "manifestVersion": 1,
  "name": "hello",
  "entrypoint": "./index.ts",
  "env": {
    "allow": ["DATABASE_URL"],
    "secretRefs": ["API_KEY"]
  },
  "network": {
    "mode": "allowlist",
    "allow": ["api.example.com:443"]
  },
  "resources": {
    "maxHeapMiB": 256,
    "cpuTimeMs": 5000,
    "wallClockTimeoutMs": 30000
  }
}
```

**Limitations of v1:**
- `entrypoint` is singular (one file per deployment)
- No `routes` section
- No route-level metadata
- No deploy flavor field
- No way to detect route conflicts at build time

### 2.3 Function Contract Today

Runtime behavior is centered on `Deno.serve()` interception.

**Current pattern (only supported):**
```typescript
import { serve } from "https://deno.land/std@x.y.z/http/server.ts";

serve(async (req) => {
  const url = new URL(req.url);

  // Users implement all routing manually
  if (url.pathname === "/api/users") {
    return new Response(JSON.stringify([...users...]));
  }

  return new Response("Not found", { status: 404 });
});
```

**Forward-looking design** (from `docs/function-contract-design.md`) proposes:
- Default function export
- Default object with HTTP methods
- Named method exports

---

## 3. Problem Statement

### 3.1 Developer Experience Issues

| Issue | Current Impact |
|-------|----------------|
| **Manual Routing** | Every function manually implements URL pattern matching, string parsing, and HTTP method dispatch |
| **No IDE Support** | Routes are opaque to editors; no autocomplete or type checking |
| **Parameter Parsing** | Manual extraction using regex/string parsing is error-prone and verbose |
| **Framework Incompatibility** | Doesn't match Next.js, Remix, SvelteKit patterns that developers expect |
| **Code Organization** | One function = one directory = monolithic bundle; doesn't scale for APIs with 10+ endpoints |
| **No Type Safety** | Parameters are strings; no validation or type checking |

### 3.2 Operational Limitations

| Issue | Current Impact |
|-------|----------------|
| **No Per-Route Metrics** | Can't identify which endpoints are slow or failing |
| **No Route Metadata** | Runtime unaware of available routes; can't validate at deploy time |
| **Route Collisions** | Not detected; silent behavior changes on updates |
| **Limited Introspection** | No way to query available routes or metrics |
| **Monolithic Deployments** | Can't split API into multiple files without multiple deployments |

### 3.3 Technical Limitations

| Issue | Current Impact |
|-------|----------------|
| **String-Based Routing** | Manual URL parsing has performance overhead |
| **Handler Interception** | `Deno.serve()` replacement couples user code to runtime |
| **Manifest Gaps** | Route information lost at deploy time; can't optimize |
| **No HTTP Method Dispatch** | All methods go to same handler in user code |

---

## 4. Vision & Success Metrics

### 4.1 The Vision

Implement a cohesive routing and function contract system that:

1. **Aligns with modern frameworks** - Developers use familiar filesystem routing patterns
2. **Reduces boilerplate** - Framework provides routing, method dispatch, and parameter extraction
3. **Improves observability** - Routes are explicit at build time; metrics are per-route
4. **Scales gracefully** - From 1 route to 1000+ routes without complexity explosion
5. **Maintains compatibility** - Existing Deno.serve() functions continue to work

### 4.2 Success Metrics

| Metric | Target |
|--------|--------|
| **Developer Friction** | 50% less code for REST APIs |
| **Framework Alignment** | Filesystem structure matches Next.js/Remix |
| **Performance** | Route matching adds < 1ms per request |
| **Adoption** | 80% of new functions use new patterns within 6 months |
| **Backward Compatibility** | 100% of existing functions work without changes |
| **Cold-Start Impact** | < 20ms additional latency |

### 4.3 Timeline

```
Phase 0: Foundation              (Weeks 1-2)   → Manifest v2, Route struct, Examples
Phase 1: Core Routing            (Weeks 3-5)   → Directory scanning, Pattern matching
Phase 2: Export Patterns         (Weeks 6-8)   → Default, Object, Named exports
Phase 3: Parameters              (Weeks 9-10)  → Dynamic segments, Type passing
Phase 4: HTTP Methods            (Weeks 11-12) → Per-method handlers, 405 support
Phase 5: Documentation & Polish  (Weeks 13-16) → Migration guide, Examples, Tooling

Total: 16 weeks to full production readiness
```

---

## 5. Proposed Architecture

### Addressing Model and Reverse Proxy Compatibility

The routing redesign must preserve the existing **two-stage runtime router** and add an optional global pre-stage, not replace the deployment prefix model.

For multi-tenant ingress by domain, the runtime also needs an optional global stage that resolves `host + path` to one deployed function namespace before the existing prefix logic.

**Stage 0: Global edge routing resolution (optional)**
- Read `Host` and request path
- Match against a global routing manifest (`routing-manifest.v1`)
- Resolve target `function_id` while keeping function isolation boundaries

**Stage 1: Deployment resolution**
- Read the first path segment
- Resolve the deployed function/app by `function_id`
- Preserve current semantics of `/{function_id}/...`

**Stage 2: Intra-app route resolution**
- Strip only the leading `/{function_id}` for internal matching
- Match the remaining suffix against the manifest route table of that deployment
- Dispatch to the correct route module or asset entry

Canonical internal example:

```text
Incoming runtime URL: /function_id_xpto/api/users/123
Deployment scope: function_id_xpto
Route suffix seen by routed app: /api/users/123
Matched route module: api/users/[id].ts
Extracted params: { id: "123" }
```

Canonical reverse proxy example:

```text
External URL:  https://function_id_xpto.my-edge-runtime.com/api/users/123
Proxy rewrite: http://localhost:9000/function_id_xpto/api/users/123
```

Canonical domain-routing example (without explicit prefix in external URL):

```text
External URL:  https://api.customer-a.com/users/123
Stage 0 match: host=api.customer-a.com, path=/users/123 -> function_id_xpto
Internal URL:  /function_id_xpto/users/123
Stage 2 match: /users/:id
```

This preserves compatibility with:

- Current local development and direct runtime access
- Subdomain-based multi-tenant ingress via reverse proxy
- Existing tooling that already assumes `function_name` or `function_id` in the first path segment

The important architectural point is: **filesystem routing augments the current addressing model; it does not remove the deployment mount prefix.**

### 5.7 Global Routing Manifest (Domain + Path)

To support domain-based routing while preserving physical separation between functions, introduce a second manifest layer at the edge/runtime boundary.

**Global manifest scope:**
- Maps `host + path` to one deployed `function_id`
- Contains no code entrypoints and no per-route JS handler metadata
- Is validated independently from function manifests

**Example shape (conceptual):**

```json
{
  "manifestVersion": 1,
  "routes": [
    {
      "host": "api.customer-a.com",
      "path": "/users/:id",
      "targetFunction": "customer-a-api"
    },
    {
      "host": "*.apps.example.com",
      "path": "/*",
      "targetFunction": "frontend-gateway"
    }
  ]
}
```

**Deterministic precedence (global layer):**
1. Exact host + exact static path
2. Exact host + dynamic path
3. Wildcard host + exact static path
4. Wildcard host + dynamic path
5. Catch-all host/path fallback

**Compatibility rule:**
- If no global route matches, fallback remains the canonical `/{function_id}/...` resolution path.

### 5.1 New Manifest Format (v2)

**Location:** `schemas/function-manifest.v2.schema.json`

```json
{
  "manifestVersion": 2,
  "name": "api-v2",
  "flavor": "routed-app",

  "routes": [
    {
      "path": "/",
      "pattern": "/",
      "entrypoint": "index.ts",
      "methods": ["GET"]
    },
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
      "parameters": [
        {
          "name": "id",
          "segment": "id",
          "optional": false,
          "catchAll": false
        }
      ]
    },
    {
      "path": "/admin/*",
      "pattern": "/admin/[...rest]",
      "entrypoint": "admin/[...rest].ts",
      "methods": ["GET"],
      "parameters": [
        {
          "name": "rest",
          "segment": "rest",
          "optional": false,
          "catchAll": true
        }
      ]
    }
  ],

  "env": {
    "allow": ["DATABASE_URL"],
    "secretRefs": ["API_KEY"]
  },
  "network": {
    "mode": "allowlist",
    "allow": ["api.example.com:443"]
  },
  "resources": {
    "maxHeapMiB": 256,
    "cpuTimeMs": 5000,
    "wallClockTimeoutMs": 30000
  }
}
```

**Key additions over v1:**
- `manifestVersion: 2` (required for routing)
- `flavor: "routed-app" | "single"` (determines how routes are treated)
- `routes: Route[]` (filesystem-discovered routes with patterns and methods)
- Route metadata for build-time validation and deploy-time introspection

**Important non-change:**
- The manifest does not replace the runtime deployment prefix
- `name` continues to identify the deployed function/app mounted under `/{name}/...`
- Route entries describe paths _inside_ that mount, for example `/api/users` means `/{function_id}/api/users` at runtime

**v1 Compatibility:**
- v1 manifests with single `entrypoint` automatically upgrade to v2 with one route at `/`

### 5.2 Supported Export Patterns

The runtime detects and supports **four mutually-exclusive patterns:**

#### Pattern 1: Default Function Export
Single function handling all methods:

```typescript
export default async function(
  req: Request,
  params?: Record<string, string>
): Promise<Response> {
  return new Response("Hello");
}
```

#### Pattern 2: HTTP Method Object
Object with method-specific handlers:

```typescript
export default {
  async GET(req: Request, params?: Record<string, string>) {
    return new Response("GET response");
  },
  async POST(req: Request, params?: Record<string, string>) {
    return new Response("Created", { status: 201 });
  },
  async ALL(req: Request, params?: Record<string, string>) {
    // Optional fallback for unsupported methods
    return new Response("Method not allowed", { status: 405 });
  }
};
```

#### Pattern 3: Named HTTP Exports
Named exports for each method:

```typescript
export async function GET(
  req: Request,
  params?: Record<string, string>
): Promise<Response> {
  return new Response("GET response");
}

export async function POST(
  req: Request,
  params?: Record<string, string>
): Promise<Response> {
  return new Response("Created", { status: 201 });
}
```

#### Pattern 4: Legacy Deno.serve() (Backward Compatible)
Existing pattern continues to work unchanged:

```typescript
import { serve } from "https://deno.land/std@x.y.z/http/server.ts";

serve(async (req) => {
  return new Response("Hello");
});
```

### 5.3 Request Flow: From Current to New

**Current (Name-Based):**
```
GET /api-v1/users/123
  ├─ Extract "api-v1" from first segment
  ├─ Lookup function "api-v1" in registry
  ├─ Rewrite path to "/users/123"
  ├─ Forward to isolate
  └─ Handler implements routing manually
```

**New (Pattern-Based, preserving deployment prefix):**
```
GET /function_id_xpto/api/users/123
  ├─ Extract and resolve deployment prefix: function_id_xpto
  ├─ Load route manifest from that deployment
  ├─ Match path against patterns: /, /api/health, /api/users, /api/users/:id, ...
  ├─ Best match: /api/users/:id (specificity: highest)
  ├─ Extract parameter: { id: "123" }
  ├─ Validate method: GET in [GET, PUT, DELETE]? Yes ✓
  ├─ Load handler for /api/users/[id].ts
  ├─ Detect export pattern → Named exports
  ├─ Call: await GET(req, { id: "123" })
  └─ Return response
```

### 5.4 Build-Time Route Generation

The CLI bundle command now generates routes:

```bash
$ thunder bundle functions/ -o api.eszip --manifest-out manifest.json
  1. Scan functions/ directory recursively
  2. For each .ts/.js file:
     - Parse filename to route pattern: [id].ts → :id
     - Load module to detect export pattern
     - Detect supported HTTP methods
     - Generate route entry
  3. Validate for route conflicts
  4. Write manifest.json with all routes
  5. Bundle into ESZIP with manifest embedded
```

**Example directory structure to routes:**
```
functions/
  index.ts                      → route: /, entrypoint: index.ts
  api/
    users.ts                    → route: /api/users, entrypoint: api/users.ts
    users/[id].ts               → route: /api/users/:id, entrypoint: api/users/[id].ts
    users/[id]/posts.ts         → route: /api/users/:id/posts
  admin/
    [...rest].ts                → route: /admin/*, entrypoint: admin/[...rest].ts
  blog/
    [[lang]].ts                 → route: /blog/:lang (optional), entrypoint: blog/[[lang]].ts
```

At runtime those routes are mounted under the deploy identifier:

```text
Deployment name: function_id_xpto
Discovered route: /api/users/[id]
Runtime URL: /function_id_xpto/api/users/123
Proxy-facing URL: https://function_id_xpto.my-edge-runtime.com/api/users/123
```

### 5.5 Route Precedence, Assets, and Frontend Behavior

To avoid ambiguity and match developer expectations from modern frameworks, routed apps need deterministic precedence rules.

**Recommended precedence inside one deployment namespace:**

1. Static asset exact match
2. Static route exact match
3. Dynamic segment routes like `/users/[id]`
4. Optional segment routes like `/blog/[[lang]]`
5. Catch-all routes like `/admin/[...rest]`
6. Explicit not-found fallback

This matters for frontend and backend coexistence in the same routed app.

Example:

```text
public/logo.svg                  -> asset route /logo.svg
routes/index.ts                  -> module route /
routes/blog/[slug].ts            -> module route /blog/:slug
routes/[...spa].ts               -> SPA fallback only if explicitly defined
```

Operational rules:

- Asset routes should short-circuit before isolate execution when the manifest marks them as static
- Frontend apps should be allowed to ship `public/` or equivalent asset trees in the same deployment artifact
- SPA fallback should never happen implicitly; it must be an explicit catch-all route or explicit framework flavor behavior
- Route collision detection must consider both module routes and asset routes

### 5.6 Deploy Artifact Model for Routed Apps

The routed deployment should still be one logical deployed function/app, but the artifact needs richer metadata than v1.

Conceptually, a `routed-app` deployment contains:

- One deploy identifier (`function_id_xpto`)
- One ESZIP bundle with all route modules
- One manifest v2 route table
- Optional static asset table for frontend files
- Optional route-level metadata such as allowed methods and cache hints

This keeps operational simplicity:

- One admin deploy call
- One mounted namespace at `/{function_id}`
- One isolate pool policy per deployed app
- One place to inspect routes and assets administratively

---

## 6. Function Flavor Support

Thunder supports two deployment modes using the same routing engine:

### 6.1 Flavor 1: Single Function (v1 Compatible)

For simple, single-endpoint functions:

```
functions/
  index.ts
```

**Manifest (auto-generated):**
```json
{
  "manifestVersion": 2,
  "name": "my-function",
  "flavor": "single",
  "routes": [
    {
      "path": "/",
      "entrypoint": "index.ts",
      "methods": ["GET", "POST"]
    }
  ]
}
```

**Function code (any pattern works):**
```typescript
// Option 1: Default function
export default async function(req: Request) {
  return new Response("Hello");
}

// Option 2: Method object
export default {
  async GET() { return new Response("GET"); },
  async POST() { return new Response("POST"); }
}

// Option 3: Named exports
export async function GET() { return new Response("GET"); }
export async function POST() { return new Response("POST"); }

// Option 4: Legacy Deno.serve()
import { serve } from "https://deno.land/std@x.y.z/http/server.ts";
serve(async (req) => new Response("Hello"));
```

### 6.2 Flavor 2: Multi-Function (Advanced)

For REST APIs with multiple endpoints:

```
functions/
  index.ts
  api/
    health.ts
    users.ts
    users/[id].ts
    users/[id]/posts.ts
    posts/[slug].ts
  admin/
    [...rest].ts
  blog/
    [[lang]].ts
```

**Manifest (auto-generated):**
```json
{
  "manifestVersion": 2,
  "name": "api-v2",
  "flavor": "routed-app",
  "routes": [
    { "path": "/", "entrypoint": "index.ts", "methods": ["GET"] },
    { "path": "/api/health", "entrypoint": "api/health.ts", "methods": ["GET"] },
    { "path": "/api/users", "entrypoint": "api/users.ts", "methods": ["GET", "POST"] },
    {
      "path": "/api/users/:id",
      "pattern": "/api/users/[id]",
      "entrypoint": "api/users/[id].ts",
      "methods": ["GET", "PUT", "DELETE"]
    }
  ]
}
```

---

## 7. Detailed Implementation Roadmap

### 7.1 Build System Changes

**File:** `crates/cli/src/lib/route_discovery.rs` (new)

Purpose: Scan filesystem and discover routes

Key functions:
- `discover_routes(functions_dir: &Path) -> Result<Vec<RouteInfo>>`
- `parse_route_from_filesystem_path(path: &Path) -> Result<RouteInfo>`
- `validate_route_conflicts(routes: &[RouteInfo]) -> Result<()>`

**File:** `crates/cli/src/lib/export_detection.rs` (new)

Purpose: Detect export patterns and supported HTTP methods

Key functions:
- `detect_export_pattern(worker: &mut Worker, module_path: &str) -> Result<ExportPattern>`
- `detect_supported_methods(worker: &mut Worker, pattern: ExportPattern) -> Result<Vec<String>>`

**File:** `crates/cli/src/commands/bundle.rs`

Changes:
- Add `--manifest-out` flag to write generated manifest
- Implement `discover_routes_and_methods()` function
- Call route discovery before bundling
- Validate routes and generate manifest
- Embed manifest in ESZIP
- Handle v1→v2 manifest migration

### 7.2 Runtime Router Changes

**File:** `crates/server/src/ingress_router.rs`

Changes:
- Add `PatternMatcher` struct with regex compilation
- Implement `match_route(path: &str, routes: &[Route]) -> Option<(Route, Params)>`
- Implement `validate_method(route: &Route, method: &str) -> Result<()>`
- Modify `route_to_function()` to use pattern matching
- Return 405 Method Not Allowed with Allow header for unsupported methods

### 7.3 Function Execution Model

**File:** `crates/runtime-core/src/isolate.rs`

Changes:
- Detect export pattern at module load time
- Implement `ExportedHandler` struct for managing different patterns
- Add method dispatch logic
- Support all four patterns (default function, method object, named exports, Deno.serve)

**File:** `crates/runtime-core/src/handlers.rs` (new)

Purpose: Handler dispatch and execution

Structs:
- `ExportedHandler` - Represents loaded module exports
- `HandlerContext` - Request context with parameters

Functions:
- `dispatch_to_handler()` - Route to correct handler based on method
- `call_default_function()` - Call default export
- `call_method_object()` - Call method from object
- `call_named_export()` - Call named function

### 7.4 Manifest Schema

**File:** `schemas/function-manifest.v2.schema.json` (new)

Purpose: JSON Schema for v2 manifests

Key fields:
- `manifestVersion: 2`
- `flavor: "single" | "routed-app"`
- `routes: Route[]` array with path patterns and methods
- Route parameters with optional/catchAll flags

**File:** `crates/runtime-core/src/manifest.rs`

Changes:
- Add `Route` struct
- Add `manifest_version` detection
- Parse both v1 and v2 formats
- Auto-upgrade v1 to v2 format
- Validate route conflicts

### 7.5 Admin API

**File:** `crates/server/src/admin_router.rs`

New endpoints:
```
GET /_internal/routes
  → List all routes from manifest

GET /_internal/routes/:path
  → Get details for specific route

PUT /_internal/routes/:path/config
  → Update per-route configuration (future)

GET /_internal/metrics/routes
  → Get per-route metrics
```

### 7.6 Global Routing Manifest and Host Resolution

**File:** `schemas/routing-manifest.v1.schema.json` (new)

Purpose: Define global host/path routing table to map requests to deployed function IDs.

**File:** `crates/server/src/ingress_router.rs`

Changes:
- Add optional Stage 0 resolver for `host + path` lookup
- Apply deterministic precedence and ambiguity detection
- Rewrite internal request target to `/{function_id}/...` after global match
- Keep current prefix-based flow as fallback when no global match exists

**File:** `crates/server/src/admin_router.rs`

New endpoints (proposed):
```
GET /_internal/routing
  → List global domain/path routing table

PUT /_internal/routing
  → Replace routing table atomically

POST /_internal/routing/validate
  → Validate conflicts and precedence before apply
```

---

## 8. File-by-File Implementation Details

This section provides concrete implementation details for each modified/new file.

### 8.1 Route Discovery (`crates/cli/src/lib/route_discovery.rs`)

```rust
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct RouteInfo {
    pub file_path: PathBuf,
    pub url_pattern: String,       // e.g., "/api/users/:id"
    pub filesystem_pattern: String, // e.g., "/api/users/[id]"
    pub dynamic_segments: Vec<RouteParameter>,
}

#[derive(Debug, Clone)]
pub struct RouteParameter {
    pub name: String,
    pub segment: String,
    pub optional: bool,
    pub catch_all: bool,
}

/// Main function to discover all routes in a directory
pub fn discover_routes(functions_dir: &Path) -> Result<Vec<RouteInfo>> {
    let mut routes = Vec::new();

    for entry in WalkDir::new(functions_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        // Skip non-TS/JS files
        if !is_route_file(path) {
            continue;
        }

        // Skip special/hidden files
        if is_special_file(path) {
            continue;
        }

        let relative = path.strip_prefix(functions_dir)?;
        let route_info = parse_route_from_filesystem_path(relative)?;
        routes.push(route_info);
    }

    // Validate before returning
    validate_route_conflicts(&routes)?;

    Ok(routes)
}

fn is_route_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e, "ts" | "tsx" | "js" | "jsx"))
        .unwrap_or(false)
}

fn is_special_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| {
            n == "_middleware.ts"
                || n == "_layout.ts"
                || n.starts_with('.')
        })
        .unwrap_or(false)
}

/// Convert filesystem path to route information
/// Examples:
///   index.ts → /
///   api/users.ts → /api/users
///   api/[id].ts → /api/:id
///   api/users/[id]/posts.ts → /api/users/:id/posts
///   admin/[...rest].ts → /admin/*
///   blog/[[lang]].ts → /blog/:lang (optional)
fn parse_route_from_filesystem_path(path: &Path) -> Result<RouteInfo> {
    let path_str = path.to_str().ok_or("Invalid path")?;
    let parts: Vec<&str> = path_str
        .split('/')
        .filter(|p| !p.is_empty())
        .collect();

    let mut url_parts = Vec::new();
    let mut parameters = Vec::new();

    for (i, part) in parts.iter().enumerate() {
        let is_last = i == parts.len() - 1;

        // Remove extension from last part
        let clean_part = if is_last {
            part.split('.').next().unwrap_or("")
        } else {
            part
        };

        if clean_part.is_empty() {
            continue;
        }

        // Parse dynamic segments: [id], [[id]], [...rest]
        if clean_part.starts_with('[') && clean_part.ends_with(']') {
            let inner = &clean_part[1..clean_part.len()-1];

            if inner.starts_with("...") {
                // Catch-all: [...rest]
                let param_name = &inner[3..];
                url_parts.push("*".to_string());
                parameters.push(RouteParameter {
                    name: param_name.to_string(),
                    segment: param_name.to_string(),
                    optional: false,
                    catch_all: true,
                });
            } else if inner.starts_with('[') && inner.ends_with(']') {
                // Optional: [[id]]
                let param_name = &inner[1..inner.len()-1];
                url_parts.push(format!(":{}", param_name));
                parameters.push(RouteParameter {
                    name: param_name.to_string(),
                    segment: param_name.to_string(),
                    optional: true,
                    catch_all: false,
                });
            } else {
                // Required: [id]
                url_parts.push(format!(":{}", inner));
                parameters.push(RouteParameter {
                    name: inner.to_string(),
                    segment: inner.to_string(),
                    optional: false,
                    catch_all: false,
                });
            }
        } else {
            // Static segment
            url_parts.push(clean_part.to_string());
        }
    }

    let url_pattern = if url_parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", url_parts.join("/"))
    };

    Ok(RouteInfo {
        file_path: path.to_path_buf(),
        url_pattern,
        filesystem_pattern: format!("/{}", parts.join("/")),
        dynamic_segments: parameters,
    })
}

fn validate_route_conflicts(routes: &[RouteInfo]) -> Result<()> {
    // Check for exact duplicate patterns
    let mut patterns = std::collections::HashSet::new();

    for route in routes {
        if !patterns.insert(route.url_pattern.clone()) {
            return Err(format!("Duplicate route: {}", route.url_pattern).into());
        }
    }

    Ok(())
}
```

### 8.2 Export Pattern Detection (`crates/cli/src/lib/export_detection.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExportPattern {
    DefaultFunction,
    MethodObject,
    NamedExports,
    DenoServe,
    Unknown,
}

/// Detect which export pattern a module uses by loading and inspecting it
pub async fn detect_export_pattern(
    worker: &mut Worker,
    module_path: &str,
) -> Result<ExportPattern> {
    // Load module to populate exports
    worker.load_main_module(module_path).await?;

    // Check for each pattern
    let pattern_script = r#"
        const module = globalThis.__esModule || {};
        const hasDefault = 'default' in module;
        const defaultValue = module.default;

        let pattern = 'unknown';

        if (typeof defaultValue === 'function') {
            pattern = 'default-function';
        } else if (typeof defaultValue === 'object' && defaultValue !== null) {
            const methods = ['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'HEAD', 'OPTIONS'];
            if (methods.some(m => typeof defaultValue[m] === 'function')) {
                pattern = 'method-object';
            }
        }

        // Check for named exports
        const methods = ['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'HEAD', 'OPTIONS'];
        if (methods.some(m => typeof module[m] === 'function')) {
            pattern = 'named-exports';
        }

        globalThis.__detected_pattern = pattern;
    "#;

    worker.execute_script("detect", pattern_script)?;

    let pattern_str = worker.execute_script(
        "get_pattern",
        "globalThis.__detected_pattern"
    )?.to_string();

    match pattern_str.as_str() {
        "default-function" => Ok(ExportPattern::DefaultFunction),
        "method-object" => Ok(ExportPattern::MethodObject),
        "named-exports" => Ok(ExportPattern::NamedExports),
        "deno-serve" => Ok(ExportPattern::DenoServe),
        _ => Ok(ExportPattern::Unknown),
    }
}

/// Detect which HTTP methods are supported by a module
pub async fn detect_supported_methods(
    worker: &mut Worker,
    pattern: ExportPattern,
) -> Result<Vec<String>> {
    let all_methods = vec![
        "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"
    ];

    match pattern {
        ExportPattern::DefaultFunction | ExportPattern::DenoServe => {
            // Default function/Deno.serve handles all methods
            Ok(all_methods.iter().map(|s| s.to_string()).collect())
        }
        ExportPattern::MethodObject => {
            let mut supported = Vec::new();

            for method in &all_methods {
                let check = format!(
                    "typeof globalThis.__esModule?.default?.{} === 'function'",
                    method
                );
                let exists = worker.execute_script("check", &check)?;

                if exists.as_bool()? {
                    supported.push(method.to_string());
                }
            }

            // Check for ALL fallback
            let has_all = worker.execute_script(
                "check_all",
                "typeof globalThis.__esModule?.default?.ALL === 'function'"
            )?;

            if has_all.as_bool()? && supported.is_empty() {
                // ALL fallback covers all methods
                supported = all_methods.iter().map(|s| s.to_string()).collect();
            }

            Ok(supported)
        }
        ExportPattern::NamedExports => {
            let mut supported = Vec::new();

            for method in &all_methods {
                let check = format!(
                    "typeof globalThis.__esModule?.{} === 'function'",
                    method
                );
                let exists = worker.execute_script("check", &check)?;

                if exists.as_bool()? {
                    supported.push(method.to_string());
                }
            }

            Ok(supported)
        }
        ExportPattern::Unknown => Ok(all_methods.iter().map(|s| s.to_string()).collect()),
    }
}
```

---

## 9. Real-World Examples

### 9.1 Example 1: Simple Greeting

**Before (Current):**
```typescript
// functions/index.ts
import { serve } from "https://deno.land/std@x.y.z/http/server.ts";

serve(async (req) => {
  return new Response("Hello, World!");
});
```

**After (Pattern 1: Default Function):**
```typescript
// functions/index.ts
export default async function(req: Request): Promise<Response> {
  return new Response("Hello, World!");
}
```

**Benefits:**
- No import needed
- Clearer intent (exported function vs. global side effect)
- Matches serverless patterns (AWS Lambda, Vercel, etc.)

### 9.2 Example 2: REST API with Multiple Methods

**Directory structure:**
```
functions/
  api/
    users.ts          # /api/users
    users/[id].ts     # /api/users/:id
```

**Implementation:**
```typescript
// functions/api/users.ts
const users = [
  { id: 1, name: "Alice" },
  { id: 2, name: "Bob" }
];

export default {
  async GET(req: Request, params?: Record<string, string>) {
    return new Response(JSON.stringify(users), {
      headers: { "content-type": "application/json" }
    });
  },

  async POST(req: Request) {
    const body = await req.json() as { name: string };
    const newUser = { id: users.length + 1, ...body };
    users.push(newUser);
    return new Response(JSON.stringify(newUser), {
      status: 201,
      headers: { "content-type": "application/json" }
    });
  }
};
```

```typescript
// functions/api/users/[id].ts
const users = [/* ... */];

export async function GET(
  req: Request,
  params: Record<string, string>
): Promise<Response> {
  const { id } = params;
  const user = users.find(u => u.id === parseInt(id));

  if (user) {
    return new Response(JSON.stringify(user), {
      headers: { "content-type": "application/json" }
    });
  }

  return new Response("Not found", { status: 404 });
}

export async function PUT(
  req: Request,
  params: Record<string, string>
): Promise<Response> {
  const { id } = params;
  const user = users.find(u => u.id === parseInt(id));

  if (user) {
    const body = await req.json() as { name: string };
    user.name = body.name;
    return new Response(JSON.stringify(user), {
      headers: { "content-type": "application/json" }
    });
  }

  return new Response("Not found", { status: 404 });
}

export async function DELETE(
  req: Request,
  params: Record<string, string>
): Promise<Response> {
  const { id } = params;
  const index = users.findIndex(u => u.id === parseInt(id));

  if (index !== -1) {
    users.splice(index, 1);
    return new Response("", { status: 204 });
  }

  return new Response("Not found", { status: 404 });
}
```

**Benefits:**
- Clear file structure mirrors API structure
- Type-safe parameter extraction
- Built-in method dispatch
- 60% less code than manual routing

### 9.3 Example 3: Blog with Optional Parameters

```
functions/
  blog/
    [[lang]].ts       # /blog or /blog/:lang
    [slug].ts         # /blog/:slug
```

```typescript
// functions/blog/[[lang]].ts
// Matches: /blog, /blog/en, /blog/fr, etc.
export async function GET(
  req: Request,
  params: Record<string, string>
): Promise<Response> {
  const lang = params.lang || 'en';
  const posts = [
    { id: 1, title: 'First Post', lang: 'en' },
    { id: 2, title: 'Premier Article', lang: 'fr' }
  ];

  const filtered = posts.filter(p => p.lang === lang);
  return new Response(JSON.stringify(filtered), {
    headers: { "content-type": "application/json" }
  });
}
```

**Key feature:** Optional parameter `lang` allows both `/blog` and `/blog/en`

### 9.4 Example 4: Admin Catch-All

```
functions/
  admin/
    [...rest].ts      # /admin/*
```

```typescript
// functions/admin/[...rest].ts
export async function GET(
  req: Request,
  params: Record<string, string>
): Promise<Response> {
  const { rest } = params;  // e.g., "users/settings"
  const url = new URL(req.url);

  // Authorization check
  const token = url.searchParams.get('token');
  if (!token || token !== 'secret') {
    return new Response('Unauthorized', { status: 401 });
  }

  // Serve content based on path
  const sections: Record<string, object> = {
    'users': { page: 'Users', count: 100 },
    'settings': { page: 'Settings', count: 0 },
    'logs': { page: 'Logs', count: 1000 }
  };

  const section = rest?.split('/')[0] || 'home';
  const content = sections[section] || { error: 'Not found' };

  return new Response(JSON.stringify(content), {
    headers: { "content-type": "application/json" }
  });
}
```

### 9.5 Example 5: Middleware & Authentication

Using higher-order functions for protected routes:

```typescript
// shared/auth.ts
export type Handler = (
  req: Request,
  params: Record<string, string>
) => Promise<Response>;

export function withAuth(handler: Handler): Handler {
  return async (req: Request, params: Record<string, string>) => {
    const auth = req.headers.get('authorization');

    if (!auth || !validateToken(auth)) {
      return new Response('Unauthorized', { status: 401 });
    }

    // Store user info for handler
    (req as any).user = extractUser(auth);

    return handler(req, params);
  };
}

function validateToken(token: string): boolean {
  return token.startsWith('Bearer ') && token.length > 10;
}

function extractUser(token: string) {
  return { id: '123', name: 'Alice' };
}
```

```typescript
// functions/api/protected.ts
import { withAuth, Handler } from '../shared/auth.ts';

const getHandler: Handler = async (req, params) => {
  const user = (req as any).user;
  return new Response(`Hello, ${user.name}!`);
};

export default {
  GET: withAuth(getHandler),
  POST: withAuth(async (req, params) => {
    const user = (req as any).user;
    const body = await req.json();
    return new Response(JSON.stringify({ user, data: body }), {
      status: 201
    });
  })
};
```

---

## 10. Phase-by-Phase Breakdown

### Phase 0: Foundation (Weeks 1-2)

**Objectives:**
1. Design route struct and manifest v2 schema
2. Create example projects
3. Write comprehensive tests for route matching

**Files:**
- `schemas/function-manifest.v2.schema.json` (new)
- `crates/cli/src/lib/route_discovery.rs` (new)
- `crates/server/src/route_matcher.rs` (new)
- Example projects (new)

**Deliverables:**
- v2 manifest schema passing JSON Schema validation
- Route matching algorithm with 100% test coverage
- 5 example projects demonstrating all patterns
- Design decision documentation

**No breaking changes** - foundation only, no runtime integration yet.

---

### Phase 1: Core Routing (Weeks 3-5)

**Objectives:**
1. Implement filesystem route discovery
2. Integrate pattern matching into ingress router
3. Add CLI support for directory-based bundling

**Files Modified:**
- `crates/cli/src/commands/bundle.rs` (add route discovery)
- `crates/server/src/ingress_router.rs` (add pattern matching)
- `crates/functions/src/registry.rs` (store routes)
- `crates/runtime-core/src/manifest.rs` (v2 schema)

**Key Features:**
- Discover all route files in directory
- Parse filesystem paths to URL patterns
- Detect route conflicts at build time
- Generate manifest with route metadata
- Match incoming requests to routes
- Extract path parameters

**Testing:**
- Unit: Route discovery, pattern parsing, matching
- Integration: Full deploy-to-invoke flow
- E2E: Multi-route deployments

**Acceptance Criteria:**
- Directory scanner finds all files correctly
- Route patterns generated from filesystem
- Route conflicts detected and reported
- Manifest v2 generated correctly
- Ingress router matches patterns
- Parameters extracted correctly

**Backward Compatibility:**
- v1 manifests still work (single entrypoint)
- Single-entrypoint deployments still work
- No breaking changes to existing APIs

---

### Phase 2: Export Patterns (Weeks 6-8)

**Objectives:**
1. Implement export pattern detection
2. Support default function export
3. Support method object export
4. Maintain Deno.serve() compatibility

**Files Modified:**
- `crates/cli/src/lib/export_detection.rs` (new)
- `crates/runtime-core/src/isolate.rs` (pattern detection)
- `crates/runtime-core/src/handlers.rs` (new)
- `crates/cli/src/commands/bundle.rs` (call detector)

**Key Features:**
- Detect export pattern at bundle time
- Support all 4 patterns
- Store pattern in manifest
- Route to appropriate handler at runtime
- Maintain backward compatibility

**Testing:**
- Unit: Pattern detection for each type
- Integration: Each pattern invoked correctly
- Compat: Deno.serve() still works

---

### Phase 3: Route Parameters (Weeks 9-10)

**Objectives:**
1. Extract dynamic parameters from paths
2. Pass parameters to handlers
3. Support optional and catch-all parameters

**Files Modified:**
- `crates/server/src/ingress_router.rs` (parameter extraction)
- `crates/runtime-core/src/handlers.rs` (parameter passing)

**Key Features:**
- Extract captured groups from regex
- Map to parameter names
- Handle optional parameters
- Handle catch-all parameters
- Pass as Record<string, string>

---

### Phase 4: HTTP Method Dispatch (Weeks 11-12)

**Objectives:**
1. Generate method metadata in manifest
2. Validate HTTP method
3. Return 405 Method Not Allowed correctly
4. Add per-method metrics

**Files Modified:**
- `crates/cli/src/lib/export_detection.rs` (method detection)
- `crates/server/src/ingress_router.rs` (method validation)
- `crates/functions/src/metrics.rs` (per-method metrics)

**Key Features:**
- Method list in manifest
- Validate method against route
- Return 405 with Allow header
- Per-method metrics
- ALL fallback support

---

### Phase 5: Documentation & Polish (Weeks 13-16)

**Objectives:**
1. Complete migration guide
2. Create comprehensive examples
3. Performance optimization
4. Edge case handling

**Deliverables:**
- Migration guide from Deno.serve() to new patterns
- Filesystem routing guide
- 5+ example projects
- Performance benchmarks
- Security guidelines
- Complete API documentation

---

## 11. Migration Strategy

### 11.1 Backward Compatibility

**v1 Manifests:**
Single entrypoint auto-upgrades to v2:
```json
// v1
{ "manifestVersion": 1, "entrypoint": "index.ts" }

// Becomes v2
{
  "manifestVersion": 2,
  "flavor": "single",
  "routes": [{ "path": "/", "entrypoint": "index.ts" }]
}
```

**Deno.serve() Functions:**
Continue to work unchanged. Runtime detects `serve()` call and routes all requests to that handler.

**Addressing compatibility:**
Existing callers that invoke functions using `http://localhost:9000/{function_id}/...` must continue to work unchanged. The migration only changes how the suffix after `/{function_id}` is interpreted inside the deployed app.

### 11.2 Deprecation Timeline

| Time | Action | User Impact |
|------|--------|------------|
| **v2.0** | New patterns available | No breaking changes |
| **v2.1 (+3 months)** | CLI deprecation warning | Optional migration |
| **v2.2 (+6 months)** | Runtime deprecation warning | Optional migration |
| **v3.0 (+12 months)** | Deno.serve() optional | Recommended upgrade |

### 11.3 Migration Assistance

Proposed CLI tooling:

```bash
# Analyze and suggest migration
thunder migrate-suggest functions/index.ts
  → Shows recommended pattern
  → Shows example code

# Auto-migrate function
thunder migrate functions/index.ts --pattern method-object
  → Updates function
  → Keeps backup

# Migrate all functions
thunder migrate-all functions/ --pattern method-object
  → Updates all functions
  → Keeps backups
  → Creates manifest
```

---

## 12. Performance Impact Analysis

### 12.1 Cold-Start Impact

**Baseline:** ~200ms (from ROADMAP.md)

**Route Matching Overhead:**
- Route discovery at startup: 5-10ms for 100 routes
- Route manifest loading: 1-2ms
- Regex compilation: 0.5-1ms per route (cached)
- **Total: ~20-30ms overhead**

**Optimizations:**
- Lazy compile regex (on first match)
- Cache compiled patterns
- Pre-index routes by specificity

**Expected Impact:** +15-20ms (acceptable)

### 12.2 Per-Request Performance

**Route Matching:**
- Pattern matching: O(n) where n = routes (linear search)
- Best match: O(1) amortized (compiled once)
- Parameter extraction: O(k) where k = parameters
- **Total: < 1ms per request**

**Benchmarks:**
```
10 routes:     < 0.1ms
100 routes:    < 0.5ms
1000 routes:   < 5ms
```

### 12.3 Memory Footprint

**Per-route overhead:**
- Route struct: ~200 bytes
- Compiled regex: 1-5 KB
- Parameter names: 50-200 bytes
- **Total: ~1-5 KB per route**

**Examples:**
- 100 routes: 200-500 KB
- 1000 routes: 2-5 MB
- 10000 routes: 20-50 MB (rare)

---

## 13. Security & Edge Cases

### 13.1 Path Parameter Validation

All parameters are strings from URL path. Sanitize before use:

```typescript
export async function GET(
  req: Request,
  params: Record<string, string>
): Promise<Response> {
  const { id } = params;

  // ✓ GOOD: Parameterized query
  const user = await db.query(
    "SELECT * FROM users WHERE id = $1",
    [id]
  );

  // ✗ BAD: String interpolation
  // const sql = `SELECT * FROM users WHERE id = ${id}`;
}
```

### 13.2 SSRF Protection

Existing allowlist model continues to work:
```json
{
  "network": {
    "mode": "allowlist",
    "allow": ["api.example.com:443"]
  }
}
```

Routing has no impact on SSRF protection.

### 13.3 Route Collision Detection

Build error on ambiguous routes:
```
functions/
  users/[id].ts
  users/[slug].ts
```

Both generate `/users/:id` pattern → **Build error**

### 13.4 Error Handling

405 Method Not Allowed response format:
```
HTTP/1.1 405 Method Not Allowed
Allow: GET, POST
Content-Length: 18
Content-Type: text/plain

Method Not Allowed
```

404 for no matching route.

---

## 14. Testing Strategy

### 14.1 Unit Tests

**Route Discovery:**
```rust
#[test]
fn test_discovers_all_files() { }

#[test]
fn test_skips_special_files() { }

#[test]
fn test_parses_static_segments() { }

#[test]
fn test_parses_dynamic_segments() { }

#[test]
fn test_parses_optional_segments() { }

#[test]
fn test_parses_catchall_segments() { }

#[test]
fn test_detects_conflicts() { }
```

**Route Matching:**
```rust
#[test]
fn test_exact_match() { }

#[test]
fn test_dynamic_match_single() { }

#[test]
fn test_optional_present() { }

#[test]
fn test_optional_absent() { }

#[test]
fn test_catchall() { }

#[test]
fn test_specificity_ordering() { }

#[test]
fn test_url_decoding() { }
```

### 14.2 Integration Tests

```rust
#[tokio::test]
async fn test_e2e_single_route() { }

#[tokio::test]
async fn test_e2e_multiple_routes() { }

#[tokio::test]
async fn test_e2e_dynamic_parameters() { }

#[tokio::test]
async fn test_e2e_method_dispatch() { }

#[tokio::test]
async fn test_e2e_deno_serve_compatibility() { }

#[tokio::test]
async fn test_e2e_preserves_function_prefix_mount() { }

#[tokio::test]
async fn test_e2e_reverse_proxy_path_rewrite_compatibility() { }
```

### 14.3 Performance Benchmarks

```rust
#[bench]
fn bench_route_matching_10() { /* target: < 0.1ms */ }

#[bench]
fn bench_route_matching_100() { /* target: < 0.5ms */ }

#[bench]
fn bench_route_matching_1000() { /* target: < 5ms */ }

#[bench]
fn bench_parameter_extraction() { /* target: < 0.05ms */ }
```

---

## 15. Open Questions & Design Decisions

### Decision 1: Bundling Strategy

**Question:** Separate bundles per route or monolithic?

**Decision:** Monolithic (Phase 1)
- Simpler implementation
- Uses existing ESZIP infrastructure
- Per-file bundling as future optimization

### Decision 2: Parameter Typing

**Question:** Typed parameters or always strings?

**Decision:** Phase 1 = strings, Phase 2+ = optional explicit typing
```typescript
// Phase 1
params: Record<string, string>

// Phase 2+
params: { id: string; slug: string }
```

### Decision 3: Middleware Support

**Question:** When to implement middleware?

**Decision:** Phase 2+ via higher-order functions (not built-in)

### Decision 4: Body Parsing

**Question:** Auto-parse JSON/form data?

**Decision:** User handles in Phase 1, helpers in Phase 2+

### Decision 5: Route Versioning

**Question:** How to version APIs?

**Decision:** Filesystem-based
```
functions/
  api/v1/users.ts
  api/v2/users.ts
```

### Decision 6: Catch-All Parameters

**Question:** What does `[...rest]` capture?

**Decision:** Without leading slash, without query string
```
Request: /admin/users/settings?page=1
Route: /admin/[...rest]
params.rest = "users/settings"
```

---

## 16. Related ROADMAP.md Tasks

Add these items to main ROADMAP.md with links to specific sections:

### High Priority

1. **Filesystem-based routing implementation** (#7)
   - Route discovery system
   - Pattern matching algorithm
   - Build-time route validation

2. **RESTful function contract** (#5.2)
   - Export pattern detection
   - HTTP method dispatch
   - Multi-pattern support

3. **Runtime router refactoring** (#7.2)
   - Pattern matching instead of name-only
   - Parameter extraction
   - 405 Method Not Allowed

4. **Manifest schema evolution (v1 → v2)** (#5.1)
   - Route definitions
   - Method configuration
   - Parameter metadata

5. **Global domain routing manifest**
  - Host/path to function mapping with deterministic precedence
  - Conflict validation across functions
  - Fallback compatibility to `/{function_id}` canonical ingress

### Medium Priority

5. **CLI bundle enhancements** (#7.1)
   - Directory scanning
   - Route discovery
   - Manifest auto-generation

6. **Admin API route introspection** (#7.5)
   - GET /_internal/routes
   - GET /_internal/metrics/routes
   - PUT /_internal/routes/:path/config

---

## Appendix A: Glossary

| Term | Definition |
|------|-----------|
| **Isolate** | V8 runtime instance with own heap and globals |
| **ESZIP** | Deno's serialized module format |
| **Manifest** | JSON config defining function routes, permissions, limits |
| **Ingress Router** | Public HTTP router accepting requests |
| **Route Pattern** | URL pattern like `/api/users/:id` |
| **Filesystem Pattern** | File structure like `/api/users/[id].ts` |
| **Export Pattern** | How function exports handler |
| **Specificity** | How exactly a pattern matches |

---

## Appendix B: Command Reference

```bash
# Bundle with route discovery
thunder bundle functions/ -o api.eszip --manifest-out manifest.json

# Deploy
thunder deploy api.eszip --name api-v1 --manifest manifest.json

# Admin API
curl http://localhost:9000/_internal/routes
curl http://localhost:9000/_internal/metrics/routes
```

---

## Conclusion

This comprehensive roadmap provides a clear, phased approach to implementing filesystem-based routing and RESTful function contracts in Thunder. The implementation:

1. **Maintains 100% backward compatibility** with existing Deno.serve()
2. **Improves developer experience** with familiar modern patterns
3. **Enables route-level observability** for better monitoring
4. **Scales from 1 to 1000+ routes** gracefully
5. **Delivers incremental value** through five independent phases

Implementation spans 16 weeks with five independently testable and deployable phases. Each phase delivers production-quality functionality with comprehensive testing and documentation.
