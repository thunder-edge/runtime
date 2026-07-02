# RESTful Function Contract Design - Complete Documentation

This directory contains the complete design specification and implementation guide for the RESTful Function Contract, a modern TypeScript-first function format for the Deno Edge Runtime.

## Quick Links

### Start Here
- **[function-contract-overview.md](function-contract-overview.md)** - Overview and navigation guide (5 min read)
- **[function-contract-quick-start.md](function-contract-quick-start.md)** - Quick reference with code examples (10 min read)

### For Understanding the Design
- **[function-contract.md](function-contract.md)** - Complete 15,000+ word specification with full details
- **[function-contract-migration.md](function-contract-migration.md)** - Before/after examples and migration patterns

### For Implementation
- **[function-contract-implementation.md](function-contract-implementation.md)** - 8-phase implementation plan with timeline

---

## Document Guide

| Document | Length | Audience | Purpose |
|----------|--------|----------|---------|
| [overview.md](function-contract-overview.md) | 5 min | Everyone | Start here - navigation and summary |
| [quick-start.md](function-contract-quick-start.md) | 10 min | Developers | Quick reference and examples |
| [migration.md](function-contract-migration.md) | 15 min | Developers | Before/after migration examples |
| [function-contract.md](function-contract.md) | 1 hour | Architects | Complete specification |
| [implementation.md](function-contract-implementation.md) | 30 min | Engineers | Implementation checklist and timeline |

---

## What is the RESTful Function Contract?

A modern TypeScript-first function format that replaces `Deno.serve()` callbacks with a cleaner, more intuitive API.

**Before:**
```ts
Deno.serve(async (req) => {
  const url = new URL(req.url);
  if (url.method === "GET" && url.pathname === "/users") {
    return new Response(JSON.stringify([]), {
      headers: { "content-type": "application/json" }
    });
  }
  return new Response(JSON.stringify({ error: "Not found" }), { status: 404 });
});
```

**After:**
```ts
import { json } from "edge://response-helpers";

export function GET(req, ctx) {
  return json([]);
}
```

---

## Key Features

✓ **30-60% less code** - No routing boilerplate
✓ **Automatic parsing** - Headers, params, query, body extracted automatically
✓ **Type-safe** - Full TypeScript support with auto-generated types
✓ **Middleware** - First-class composition support
✓ **Filesystem routing** - Routes determined by file structure
✓ **Standard responses** - Response helpers for JSON, errors, HTML, redirects
✓ **Backwards compatible** - Old `Deno.serve()` format still works
✓ **Minimal overhead** - <2ms cold-start impact

---

## Three Supported Patterns

### Pattern 1: Single-Method Export
```ts
export function GET(req, ctx) {
  return json({ message: "Hello" });
}
```

### Pattern 2: Multi-Method Handler
```ts
export default {
  GET(req, ctx) { /* ... */ },
  POST(req, ctx) { /* ... */ },
  DELETE(req, ctx) { /* ... */ }
}
```

### Pattern 3: Default Handler
```ts
export default function handler(req, ctx) {
  // Complex routing logic
}
```

---

## FunctionContext Object

Every handler receives a `FunctionContext` with:

```ts
interface FunctionContext {
  params: Record<string, string>;      // URL path params: /users/[id]
  query: Record<string, string>;        // Query string: ?foo=bar
  headers: Headers;                     // HTTP headers
  method: string;                       // GET, POST, PUT, etc.
  url: URL;                            // Full URL object
  body?: unknown;                      // Auto-parsed JSON, FormData, etc.
  cookies: Record<string, string>;     // Parsed cookies
  route: string;                       // Route template
  metadata: Record<string, unknown>;   // For middleware
}
```

---

## Response Helpers

Standard functions for common responses:

```ts
// JSON response
json({ message: "Hello" })

// Error response
error("Not found", { status: 404 })

// HTML response
html("<h1>Hello</h1>")

// Redirect
redirect("/new-path")

// Streaming
stream(readableStream)

// Binary file
file(uint8array, { headers: { "content-type": "image/png" } })
```

For the implemented runtime helper module and generic auto-response adapter, see:
- [http-response-helpers.md](http-response-helpers.md) (`thunder:http`)

---

## Middleware Support

```ts
const authMiddleware = async (req, ctx, next) => {
  const token = ctx.headers.get("authorization");
  if (!token) {
    return error("Unauthorized", { status: 401 });
  }
  ctx.metadata.user = { id: "user-1" };
  return await next();
};

export const middleware = [authMiddleware];

export function GET(req, ctx) {
  const user = ctx.metadata.user;  // Set by middleware
  return json({ user });
}
```

---

## Filesystem Routing

Routes determined by file structure:

```
functions/
├── users.ts                  → GET /users
├── users/[id].ts            → GET /users/123
├── api/search.ts            → GET /api/search
└── static/[...file].ts      → GET /static/any/path
```

---

## Implementation Timeline

| Phase | Duration | Focus |
|-------|----------|-------|
| 1 | 2 weeks | Core types, detection, helpers |
| 2 | 1 week | Request processing & parsing |
| 3 | 1 week | Dispatch & error handling |
| 4 | 1 week | Middleware & hooks |
| 5 | 1 week | Type generation & docs |
| 6 | 1-2 weeks | Testing & validation |
| 7 | 1 week | Backwards compatibility |
| 8 | 1 week | Release & examples |
| **Total** | **8-9 weeks** | **v2.0 Ready** |

---

## Real-World Examples

### REST API
```ts
// functions/users/[id].ts
export default {
  GET(req, ctx) {
    return json({ userId: ctx.params.id });
  },
  PUT(req, ctx) {
    const updates = ctx.body;
    return json({ updated: true, ...updates });
  },
  DELETE(req, ctx) {
    return json({ deleted: true });
  }
}
```

### Form Handling
```ts
// functions/contact.ts
export const GET = (req, ctx) => {
  return html("<form method='POST'>...</form>");
};

export const POST = async (req, ctx) => {
  const form = ctx.body as FormData;
  const email = form.get("email");
  return json({ received: email });
};
```

### Protected Route
```ts
// functions/api/profile.ts
const authMiddleware = async (req, ctx, next) => {
  const token = ctx.headers.get("authorization");
  if (!token) return error("Unauthorized", { status: 401 });
  ctx.metadata.userId = verifyToken(token);
  return await next();
};

export const middleware = [authMiddleware];

export const GET = (req, ctx) => {
  return json({ userId: ctx.metadata.userId });
};
```

See [function-contract-migration.md](function-contract-migration.md) for 7+ complete examples.

---

## Backwards Compatibility

- Old `Deno.serve()` format still works ✓
- Both formats can coexist in same deployment ✓
- Gradual migration path planned ✓
- No breaking changes in v2.0 ✓
- Deprecation warnings in v2.1+ ✓

---

## For Different Audiences

### I'm a Developer (New or Existing)
👉 Start: [function-contract-quick-start.md](function-contract-quick-start.md)
Then: [function-contract-migration.md](function-contract-migration.md)

### I'm an Architect/Decision-Maker
👉 Start: [function-contract-overview.md](function-contract-overview.md)
Then: [function-contract.md](function-contract.md) (Design Philosophy section)

### I'm Implementing This Feature
👉 Start: [function-contract-implementation.md](function-contract-implementation.md)
Then: [function-contract.md](function-contract.md) (Implementation Guide section)

### I Want All the Details
👉 Read: [function-contract.md](function-contract.md) (Complete specification)

---

## Key Numbers

| Metric | Value |
|--------|-------|
| Code reduction | 30-60% |
| Cold-start overhead | <2ms |
| Context size | ~1.5KB |
| Implementation timeline | 8-9 weeks |
| Test coverage target | >90% |
| Performance regression | <1% |

---

## Comparison with Other Platforms

| Platform | Multi-Method | Context | Middleware | Type Safety | Routing |
|----------|--------------|---------|------------|------------|---------|
| Vercel | Poor | Basic | No | Good | Config |
| Cloudflare | Manual | Basic | No | Partial | Manual |
| AWS Lambda | Manual | Good | No | Good | Config |
| New Thunder | Excellent | Excellent | Yes | Excellent | Filesystem |

---

## Success Criteria

- [ ] >90% test coverage
- [ ] <2ms cold-start overhead
- [ ] <2KB context memory footprint
- [ ] 10+ working examples
- [ ] Comprehensive documentation
- [ ] >50% adoption in 6 months
- [ ] Positive developer feedback

---

## Document Organization

```
docs/
├── FUNCTION_CONTRACT_README.md    ← YOU ARE HERE
├── function-contract-overview.md  - Overview & navigation
├── function-contract-quick-start.md - Quick reference
├── function-contract-migration.md - Migration examples
├── function-contract.md           - Full specification
└── function-contract-implementation.md - Implementation plan
```

---

## Next Steps

### To Learn More
1. Read [function-contract-overview.md](function-contract-overview.md) (5 min)
2. Try examples in [function-contract-quick-start.md](function-contract-quick-start.md) (10 min)
3. Review migration examples in [function-contract-migration.md](function-contract-migration.md) (15 min)

### To Implement
1. Review [function-contract-implementation.md](function-contract-implementation.md)
2. Follow the 8-phase implementation plan
3. Use provided test matrices and checklists
4. Reference [function-contract.md](function-contract.md) for technical details

### To Migrate Functions
1. Pick a function
2. Choose a pattern (single-method, multi-method, or default)
3. Follow examples in [function-contract-migration.md](function-contract-migration.md)
4. Test with `thunder watch`
5. Deploy with `thunder bundle`

---

## Questions?

Refer to:
- **Quick questions?** → [function-contract-quick-start.md](function-contract-quick-start.md)
- **Migration help?** → [function-contract-migration.md](function-contract-migration.md)
- **Implementation details?** → [function-contract.md](function-contract.md)
- **How to build it?** → [function-contract-implementation.md](function-contract-implementation.md)

---

**Version:** 1.0
**Status:** Design Complete - Ready for Implementation
**Date:** 2026-03-07
**Estimated v2.0 Release:** Q2 2026 (9 weeks from start)
