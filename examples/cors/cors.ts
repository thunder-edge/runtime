// Example: CORS (Cross-Origin Resource Sharing)
// Demonstrates CORS header handling

Deno.serve(async (req) => {
  const url = new URL(req.url);

  // CORS configuration
  const ALLOWED_ORIGINS = [
    "http://localhost:3000",
    "http://localhost:8000",
    "https://example.com",
  ];
  const ALLOWED_METHODS = ["GET", "POST", "OPTIONS", "PUT", "DELETE"];
  const ALLOWED_HEADERS = [
    "content-type",
    "authorization",
    "x-custom-header",
  ];

  // Get origin from request
  const origin = req.headers.get("origin");
  const isAllowedOrigin =
    origin && ALLOWED_ORIGINS.includes(origin);

  // Helper to add CORS headers
  const addCorsHeaders = (headers: Headers) => {
    if (isAllowedOrigin && origin) {
      headers.set("access-control-allow-origin", origin);
      headers.set("access-control-allow-credentials", "true");
      headers.set(
        "access-control-allow-methods",
        ALLOWED_METHODS.join(", ")
      );
      headers.set(
        "access-control-allow-headers",
        ALLOWED_HEADERS.join(", ")
      );
      headers.set("access-control-max-age", "3600");
    }
  };

  // Handle preflight OPTIONS request
  if (req.method === "OPTIONS") {
    const headers = new Headers();
    addCorsHeaders(headers);
    return new Response(null, {
      status: 204,
      headers,
    });
  }

  // Main API routes
  if (url.pathname === "/api/data") {
    const headers = new Headers({
      "content-type": "application/json",
    });
    addCorsHeaders(headers);

    const response = {
      message: "This data is CORS-enabled",
      origin: origin || "no origin",
      timestamp: new Date().toISOString(),
      data: [
        { id: 1, value: "Item 1" },
        { id: 2, value: "Item 2" },
        { id: 3, value: "Item 3" },
      ],
    };

    return new Response(JSON.stringify(response), { headers });
  }

  // POST endpoint
  if (url.pathname === "/api/submit" && req.method === "POST") {
    const headers = new Headers({
      "content-type": "application/json",
    });
    addCorsHeaders(headers);

    try {
      const body = await req.json();
      return new Response(
        JSON.stringify({
          success: true,
          message: "Data received",
          received: body,
        }),
        { headers }
      );
    } catch {
      return new Response(
        JSON.stringify({ error: "Invalid JSON" }),
        { status: 400, headers }
      );
    }
  }

  // Default: HTML page with CORS examples
  const html = `
    <!DOCTYPE html>
    <html>
    <head>
      <title>CORS Example</title>
      <style>
        body {
          font-family: Arial;
          max-width: 900px;
          margin: 0 auto;
          padding: 40px 20px;
          background: #f5f5f5;
        }
        .container {
          background: white;
          padding: 30px;
          border-radius: 8px;
          box-shadow: 0 2px 10px rgba(0,0,0,0.1);
        }
        h1, h2 {
          color: #333;
        }
        button {
          background: #667eea;
          color: white;
          border: none;
          padding: 12px 24px;
          border-radius: 4px;
          cursor: pointer;
          margin: 10px 5px 10px 0;
          font-size: 1em;
        }
        button:hover {
          background: #764ba2;
        }
        #result {
          background: #f9f9f9;
          padding: 20px;
          border-radius: 4px;
          margin-top: 20px;
          border-left: 4px solid #667eea;
        }
        pre {
          background: #f0f0f0;
          padding: 10px;
          border-radius: 4px;
          overflow-x: auto;
        }
        .info {
          background: #e8f4f8;
          padding: 15px;
          border-radius: 4px;
          margin: 20px 0;
          border-left: 4px solid #4a90e2;
        }
      </style>
    </head>
    <body>
      <div class="container">
        <h1>🔓 CORS Example</h1>

        <div class="info">
          <strong>Note:</strong> To test CORS, you need to make requests from a different origin.
          Try using curl or a REST client with the origin header.
        </div>

        <h2>Available Endpoints:</h2>

        <h3>GET /api/data</h3>
        <button onclick="testGetRequest()">Test GET Request</button>

        <h3>POST /api/submit</h3>
        <button onclick="testPostRequest()">Test POST Request</button>

        <div id="result"></div>

        <h2>Using curl:</h2>
        <pre>curl -H "Origin: http://localhost:8000" https://your-edge-function.example.com/api/data</pre>
      </div>

      <script>
        async function testGetRequest() {
          const resultDiv = document.getElementById('result');
          try {
            const response = await fetch('/api/data');
            const data = await response.json();
            resultDiv.innerHTML = '<strong>GET Response:</strong><pre>' + JSON.stringify(data, null, 2) + '</pre>';
          } catch (error) {
            resultDiv.innerHTML = '<strong>Error:</strong> ' + error.message;
          }
        }

        async function testPostRequest() {
          const resultDiv = document.getElementById('result');
          try {
            const response = await fetch('/api/submit', {
              method: 'POST',
              headers: {
                'Content-Type': 'application/json',
              },
              body: JSON.stringify({ message: 'Hello from CORS test' })
            });
            const data = await response.json();
            resultDiv.innerHTML = '<strong>POST Response:</strong><pre>' + JSON.stringify(data, null, 2) + '</pre>';
          } catch (error) {
            resultDiv.innerHTML = '<strong>Error:</strong> ' + error.message;
          }
        }
      </script>
    </body>
    </html>
  `;

  const headers = new Headers({
    "content-type": "text/html; charset=utf-8",
  });

  return new Response(html, { headers });
});
