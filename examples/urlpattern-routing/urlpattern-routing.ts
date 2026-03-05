// Example: URLPattern - Advanced Routing
// Demonstrates URLPattern API for complex URL matching and routing

Deno.serve((req) => {
  const url = new URL(req.url);

  // Define routes using URLPattern
  // URLPattern allows complex, declarative routing

  // Pattern 1: /api/users/:id
  const userPattern = new URLPattern({
    pathname: "/api/users/:id",
  });

  // Pattern 2: /files/:path*
  const filesPattern = new URLPattern({
    pathname: "/files/:path*",
  });

  // Pattern 3: /products/:category/:productId
  const productPattern = new URLPattern({
    pathname: "/products/:category/:productId",
  });

  // Pattern 4: /search with query parameters
  const searchPattern = new URLPattern({
    pathname: "/search",
  });

  // Pattern 5: Complex subdomain matching
  const apiVersionPattern = new URLPattern({
    pathname: "/api/v:version/resource/:id",
  });

  // Try to match patterns
  let match = null;

  if (url.pathname === "/" && req.method === "GET") {
    const html = `
      <!DOCTYPE html>
      <html>
      <head>
        <meta charset="UTF-8">
        <title>URLPattern Routing</title>
        <style>
          * { margin: 0; padding: 0; box-sizing: border-box; }
          body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #f5f5f5; padding: 40px 20px; }
          .container { max-width: 1000px; margin: 0 auto; background: white; border-radius: 8px; padding: 40px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
          h1 { color: #333; margin-bottom: 10px; }
          .subtitle { color: #999; margin-bottom: 30px; }
          h2 { color: #667eea; margin: 30px 0 15px; font-size: 1.2em; }
          .example { background: #f9f9f9; border-left: 4px solid #667eea; padding: 15px; margin: 10px 0; border-radius: 4px; }
          code { background: #f0f0f0; padding: 2px 6px; border-radius: 3px; font-family: monospace; }
          a { color: #667eea; text-decoration: none; margin-right: 10px; }
          a:hover { text-decoration: underline; }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>🗂️ URLPattern - Advanced Routing</h1>
          <p class="subtitle">Explore URLPattern API for declarative URL matching</p>

          <h2>Try These Routes:</h2>

          <div class="example">
            <p><strong>Pattern:</strong> <code>/api/users/:id</code></p>
            <p>
              <a href="/api/users/123">User 123</a>
              <a href="/api/users/alice">User alice</a>
            </p>
          </div>

          <div class="example">
            <p><strong>Pattern:</strong> <code>/files/:path*</code></p>
            <p>
              <a href="/files/docs/readme.md">docs/readme.md</a>
              <a href="/files/images/photo/2024/pic.jpg">images/photo/2024/pic.jpg</a>
            </p>
          </div>

          <div class="example">
            <p><strong>Pattern:</strong> <code>/products/:category/:productId</code></p>
            <p>
              <a href="/products/electronics/laptop-001">electronics/laptop-001</a>
              <a href="/products/books/novel-042">books/novel-042</a>
            </p>
          </div>

          <div class="example">
            <p><strong>Pattern:</strong> <code>/search?q=:term</code></p>
            <p>
              <a href="/search?q=javascript">Search: javascript</a>
              <a href="/search?q=deno&lang=en">Search: deno (with lang)</a>
            </p>
          </div>

          <div class="example">
            <p><strong>Pattern:</strong> <code>/api/v:version/resource/:id</code></p>
            <p>
              <a href="/api/v1/resource/123">API v1 - resource 123</a>
              <a href="/api/v2/resource/456">API v2 - resource 456</a>
            </p>
          </div>

          <h2>Route Matching System</h2>
          <p style="line-height: 1.8; color: #666; margin: 15px 0;">
            URLPattern provides:
          </p>
          <ul style="margin-left: 20px; line-height: 1.8; color: #666;">
            <li>Named parameters (e.g., :id, :category)</li>
            <li>Wildcard patterns (e.g., :path*)</li>
            <li>Multiple pathname segments</li>
            <li>Query parameter parsing</li>
            <li>Hash fragment matching</li>
            <li>RegExp-based optional matching</li>
          </ul>
        </div>
      </body>
      </html>
    `;
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }

  // Test user pattern
  match = userPattern.test(url);
  if (match) {
    const result = userPattern.exec(url);
    return new Response(
      JSON.stringify({
        pattern: "/api/users/:id",
        matched: true,
        groups: result?.pathname.groups,
        id: result?.pathname.groups?.id,
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  // Test files pattern
  match = filesPattern.test(url);
  if (match) {
    const result = filesPattern.exec(url);
    return new Response(
      JSON.stringify({
        pattern: "/files/:path*",
        matched: true,
        path: result?.pathname.groups?.path,
        fullMatch: result?.pathname.input,
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  // Test product pattern
  match = productPattern.test(url);
  if (match) {
    const result = productPattern.exec(url);
    return new Response(
      JSON.stringify({
        pattern: "/products/:category/:productId",
        matched: true,
        category: result?.pathname.groups?.category,
        productId: result?.pathname.groups?.productId,
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  // Test search with query params
  if (url.pathname === "/search") {
    const query = url.searchParams.get("q") || "";
    const lang = url.searchParams.get("lang") || "en";

    return new Response(
      JSON.stringify({
        pattern: "/search?q=:term",
        matched: true,
        query: query,
        language: lang,
        timestamp: new Date().toISOString(),
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  // Test API version pattern
  match = apiVersionPattern.test(url);
  if (match) {
    const result = apiVersionPattern.exec(url);
    return new Response(
      JSON.stringify({
        pattern: "/api/v:version/resource/:id",
        matched: true,
        version: result?.pathname.groups?.version,
        id: result?.pathname.groups?.id,
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  return new Response("Not found", { status: 404 });
});
