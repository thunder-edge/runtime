// Example: Request/Response Modification
// Demonstrates modifying request and response properties

Deno.serve(async (req) => {
  const url = new URL(req.url);

  // Add custom headers to modify the request
  const headers = new Headers(req.headers);
  headers.set("x-processed-at", new Date().toISOString());
  headers.set("x-request-id", crypto.randomUUID());

  // Create a new request with modified headers
  const modifiedReq = new Request(req, {
    headers: headers,
  });

  // Inspect original vs modified headers
  if (url.pathname === "/headers") {
    const responseHeaders: Record<string, unknown> = {
      original: {
        "user-agent": req.headers.get("user-agent"),
        "accept": req.headers.get("accept"),
        "host": req.headers.get("host"),
      },
      added: {
        "x-processed-at": modifiedReq.headers.get("x-processed-at"),
        "x-request-id": modifiedReq.headers.get("x-request-id"),
      },
    };

    return new Response(JSON.stringify(responseHeaders, null, 2), {
      headers: { "content-type": "application/json" },
    });
  }

  // Modify response
  if (url.pathname === "/modify-response") {
    const originalResponse = new Response(
      JSON.stringify({ message: "Original response" }),
      {
        status: 200,
        headers: { "content-type": "application/json" },
      }
    );

    // Clone the response to read the body
    const cloned = originalResponse.clone();
    const data = await cloned.json();

    // Create new response with modified data
    const modifiedData = {
      ...data,
      modified: true,
      timestamp: new Date().toISOString(),
      requestId: modifiedReq.headers.get("x-request-id"),
    };

    return new Response(JSON.stringify(modifiedData, null, 2), {
      status: 200,
      headers: {
        "content-type": "application/json",
        "x-modified": "true",
        "cache-control": "no-cache",
      },
    });
  }

  // Transform response based on accept header
  if (url.pathname === "/transform") {
    const accept = req.headers.get("accept") || "application/json";
    const data = {
      title: "Transformed Response",
      content: "This response can be transformed based on Accept header",
      timestamp: new Date().toISOString(),
    };

    if (accept.includes("application/json")) {
      return new Response(JSON.stringify(data, null, 2), {
        headers: { "content-type": "application/json" },
      });
    }

    if (accept.includes("text/plain")) {
      const text = `
Title: ${data.title}
Content: ${data.content}
Timestamp: ${data.timestamp}
      `.trim();
      return new Response(text, {
        headers: { "content-type": "text/plain" },
      });
    }

    if (accept.includes("text/html")) {
      const html = `
        <!DOCTYPE html>
        <html>
        <head>
          <title>${data.title}</title>
          <style>
            body { font-family: Arial; padding: 40px; background: #f5f5f5; }
            .container { background: white; padding: 30px; border-radius: 8px; max-width: 600px; margin: 0 auto; }
          </style>
        </head>
        <body>
          <div class="container">
            <h1>${data.title}</h1>
            <p>${data.content}</p>
            <small>${data.timestamp}</small>
          </div>
        </body>
        </html>
      `;
      return new Response(html, {
        headers: { "content-type": "text/html; charset=utf-8" },
      });
    }

    // Default to JSON
    return new Response(JSON.stringify(data, null, 2), {
      headers: { "content-type": "application/json" },
    });
  }

  // Home page
  const html = `
    <!DOCTYPE html>
    <html>
    <head>
      <title>Request/Response Modification</title>
      <style>
        body { font-family: Arial; padding: 40px; background: #f5f5f5; }
        .container { max-width: 800px; margin: 0 auto; background: white; padding: 30px; border-radius: 8px; }
        h1 { color: #333; }
        .example { background: #f9f9f9; padding: 15px; margin: 15px 0; border-left: 4px solid #667eea; }
        code { background: #f0f0f0; padding: 2px 6px; border-radius: 3px; }
        a { color: #667eea; text-decoration: none; }
        a:hover { text-decoration: underline; }
      </style>
    </head>
    <body>
      <div class="container">
        <h1>🔧 Request/Response Modification</h1>

        <h2>Examples:</h2>

        <div class="example">
          <p><strong>View Headers:</strong></p>
          <p><code>GET /headers</code> → <a href="/headers">Check headers</a></p>
        </div>

        <div class="example">
          <p><strong>Modify Response:</strong></p>
          <p><code>GET /modify-response</code> → <a href="/modify-response">View modified</a></p>
        </div>

        <div class="example">
          <p><strong>Transform by Accept Header:</strong></p>
          <p><code>GET /transform</code></p>
          <p><a href="/transform">JSON</a> | <a href="/transform" onclick="fetch('/transform', {headers: {'Accept': 'text/plain'}}).then(r => r.text()).then(t => alert(t));">Text</a> (check console)</p>
        </div>

        <h2>Capabilities:</h2>
        <ul style="line-height: 1.8;">
          <li>Add custom headers to requests</li>
          <li>Modify response content and headers</li>
          <li>Transform response format based on Accept header</li>
          <li>Clone and read response bodies</li>
          <li>Generate unique request IDs</li>
        </ul>
      </div>
    </body>
    </html>
  `;

  return new Response(html, {
    headers: { "content-type": "text/html; charset=utf-8" },
  });
});
