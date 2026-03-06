// Example: Middleware Pattern
// Demonstrates composable middleware functions

// Middleware interface
type Middleware = (
  req: Request,
  next: MiddlewareNext
) => Promise<Response>;
type MiddlewareNext = () => Promise<Response>;

// Middleware: Logging
const loggingMiddleware = (req: Request, next: MiddlewareNext): Promise<Response> => {
  const startTime = performance.now();
  console.log(`[${new Date().toISOString()}] ${req.method} ${new URL(req.url).pathname}`);

  const response = Promise.resolve(next());
  response.then((res) => {
    const duration = performance.now() - startTime;
    console.log(`  → Status: ${res.status}, Duration: ${duration.toFixed(2)}ms`);
  });

  return response;
};

// Middleware: Authentication
const authMiddleware = async (req: Request, next: MiddlewareNext): Promise<Response> => {
  const token = req.headers.get("authorization")?.replace("Bearer ", "");

  if (!token) {
    return new Response(
      JSON.stringify({ error: "Missing authorization token" }),
      {
        status: 401,
        headers: { "content-type": "application/json" },
      }
    );
  }

  if (token !== "secret-token-123") {
    return new Response(
      JSON.stringify({ error: "Invalid token" }),
      {
        status: 403,
        headers: { "content-type": "application/json" },
      }
    );
  }

  return await next();
};

// Middleware: Compression header detection
const compressionMiddleware = (req: Request, next: MiddlewareNext): Promise<Response> => {
  const response = Promise.resolve(next());
  return response.then((res) => {
    const accept = req.headers.get("accept-encoding") || "";
    const headers = new Headers(res.headers);

    if (accept.includes("gzip")) {
      headers.set("content-encoding", "gzip");
    }

    return new Response(res.body, {
      status: res.status,
      headers,
    });
  });
};

// Middleware: CORS
const corsMiddleware = async (req: Request, next: MiddlewareNext): Promise<Response> => {
  if (req.method === "OPTIONS") {
    return new Response(null, {
      status: 204,
      headers: {
        "access-control-allow-origin": "*",
        "access-control-allow-methods": "GET, POST, OPTIONS",
        "access-control-allow-headers": "content-type, authorization",
      },
    });
  }

  const response = await next();
  const headers = new Headers(response.headers);
  headers.set("access-control-allow-origin", "*");
  return new Response(response.body, {
    status: response.status,
    headers,
  });
};

// Middleware composer
function compose(...middlewares: Middleware[]) {
  return async (req: Request, handler: MiddlewareNext): Promise<Response> => {
    let index = -1;

    const dispatch = async (i: number): Promise<Response> => {
      if (i <= index) {
        throw new Error("Middleware called multiple times");
      }
      index = i;

      if (i === middlewares.length) {
        return await handler();
      }

      const middleware = middlewares[i];
      return await middleware(req, () => dispatch(i + 1));
    };

    return dispatch(0);
  };
}

// Route handler
const router = (req: Request): Response => {
  const url = new URL(req.url);

  if (url.pathname === "/") {
    const html = `
      <!DOCTYPE html>
      <html>
      <head>
        <title>Middleware Pattern</title>
        <style>
          body { font-family: Arial; padding: 40px; background: #f5f5f5; }
          .container { max-width: 800px; margin: 0 auto; background: white; padding: 30px; border-radius: 8px; }
          h1 { color: #333; }
          h2 { color: #667eea; margin-top: 30px; }
          .example { background: #f9f9f9; padding: 15px; margin: 15px 0; border-left: 4px solid #667eea; }
          code { background: #f0f0f0; padding: 2px 6px; border-radius: 3px; }
          button { background: #667eea; color: white; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; margin-top: 10px; }
          button:hover { background: #764ba2; }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>🔗 Middleware Pattern</h1>
          <p>This example demonstrates composable middleware functions.</p>

          <h2>Applied Middleware:</h2>
          <ul style="line-height: 1.8; color: #666;">
            <li>✓ Logging (logs all requests)</li>
            <li>✓ CORS (handles CORS headers)</li>
            <li>✓ Auth (validates tokens on /api endpoints)</li>
            <li>✓ Compression detection (checks Accept-Encoding)</li>
          </ul>

          <h2>Try These:</h2>
          <div class="example">
            <p><strong>GET /public</strong> - No auth required</p>
            <button onclick="fetch('/public').then(r => r.json()).then(d => alert(JSON.stringify(d, null, 2)))">Try Request</button>
          </div>

          <div class="example">
            <p><strong>GET /api/data</strong> - Requires token</p>
            <button onclick="fetch('/api/data', {headers: {'Authorization': 'Bearer secret-token-123'}}).then(r => r.json()).then(d => alert(JSON.stringify(d, null, 2)))">With Valid Token</button>
            <button onclick="fetch('/api/data').then(r => r.json()).then(d => alert(JSON.stringify(d, null, 2)))">Without Token</button>
          </div>

          <h2>How Middleware Works:</h2>
          <ol style="line-height: 1.8; color: #666;">
            <li>Request comes in</li>
            <li>Passes through each middleware in order</li>
            <li>Each middleware can modify request/response</li>
            <li>Handler processes the request</li>
            <li>Response returns through middlewares</li>
          </ol>

          <p style="margin-top: 30px; color: #999;">Check browser console to see logging middleware output</p>
        </div>
      </body>
      </html>
    `;
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }

  if (url.pathname === "/public") {
    return new Response(
      JSON.stringify({
        message: "Public endpoint",
        timestamp: new Date().toISOString(),
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  if (url.pathname === "/api/data") {
    return new Response(
      JSON.stringify({
        message: "Protected data",
        data: [
          { id: 1, name: "Item 1" },
          { id: 2, name: "Item 2" },
        ],
        timestamp: new Date().toISOString(),
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  return new Response("Not found", { status: 404 });
};

// Create middleware chain
const middlewareChain = compose(
  loggingMiddleware,
  corsMiddleware,
  authMiddleware
);

// Main server
Deno.serve(async (req) => {
  const url = new URL(req.url);

  // Routes that don't need auth
  if (
    url.pathname === "/" ||
    url.pathname === "/public"
  ) {
    return compose(loggingMiddleware, corsMiddleware)(req, () =>
      Promise.resolve(router(req))
    );
  }

  // Protected routes
  if (url.pathname.startsWith("/api")) {
    return middlewareChain(req, () => Promise.resolve(router(req)));
  }

  return router(req);
});
