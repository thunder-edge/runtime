// Example: Documentation & Examples Index
// Interactive documentation page with all examples, categories, and descriptions

interface Example {
  id: string;
  name: string;
  category: string;
  description: string;
  features: string[];
  apis: string[];
  difficulty: "beginner" | "intermediate" | "advanced";
  path: string;
}

const examples: Example[] = [
  {
    id: "hello",
    name: "Hello World",
    category: "Basics",
    description: "Simple edge function that returns a basic HTTP response",
    features: ["Response", "Plain text"],
    apis: ["Deno.serve"],
    difficulty: "beginner",
    path: "/hello",
  },
  {
    id: "http-request",
    name: "HTTP Requests",
    category: "Networking",
    description: "Make HTTP requests to external APIs and process responses",
    features: ["Fetch API", "JSON parsing", "Error handling"],
    apis: ["fetch", "Response", "JSON"],
    difficulty: "beginner",
    path: "/http-request",
  },
  {
    id: "html-page",
    name: "HTML Page",
    category: "Rendering",
    description: "Serve a complete HTML page with styling and layout",
    features: ["HTML generation", "CSS styling", "Responsive design"],
    apis: ["Response", "Templates"],
    difficulty: "beginner",
    path: "/html-page",
  },
  {
    id: "json-api",
    name: "REST JSON API",
    category: "APIs",
    description: "Build a RESTful API with JSON endpoints, routing, and CORS",
    features: ["Routing", "CORS", "HTTP methods", "Error handling"],
    apis: ["Request", "Response", "JSON"],
    difficulty: "intermediate",
    path: "/json-api",
  },
  {
    id: "streaming",
    name: "Streaming Data",
    category: "Streams",
    description: "Stream data to clients using ReadableStream and NDJSON",
    features: ["ReadableStream", "Streaming", "Data generation"],
    apis: ["ReadableStream", "Response"],
    difficulty: "intermediate",
    path: "/streaming-data",
  },
  {
    id: "websocket",
    name: "WebSocket",
    category: "Networking",
    description: "Real-time bidirectional communication with WebSocket",
    features: ["WebSocket", "Async iteration", "Message handling"],
    apis: ["Deno.upgrade", "WebSocket"],
    difficulty: "advanced",
    path: "/websocket",
  },
  {
    id: "basic-auth",
    name: "Basic Authentication",
    category: "Security",
    description: "Implement HTTP Basic Auth with timing-safe comparison",
    features: ["HTTP Auth", "Base64 decoding", "Timing-safe comparison"],
    apis: ["Headers", "WebCrypto"],
    difficulty: "intermediate",
    path: "/basic-auth",
  },
  {
    id: "preact-ssr",
    name: "Preact SSR",
    category: "Rendering",
    description: "Server-side rendering of Preact components",
    features: ["Component rendering", "HTML generation", "API integration"],
    apis: ["String templates", "Response"],
    difficulty: "intermediate",
    path: "/preact-ssr",
  },
  {
    id: "wasm",
    name: "WebAssembly",
    category: "Advanced",
    description: "Execute WebAssembly modules for high-performance computation",
    features: ["WAT compilation", "WASM execution", "Module instantiation"],
    apis: ["WebAssembly", "Uint8Array"],
    difficulty: "advanced",
    path: "/wasm",
  },
  {
    id: "url-redirect",
    name: "URL Redirects",
    category: "Routing",
    description: "URL redirects, URL rewriting, and language detection",
    features: ["Redirects", "URL rewriting", "Header detection"],
    apis: ["URL", "Response"],
    difficulty: "beginner",
    path: "/url-redirect",
  },
  {
    id: "cors",
    name: "CORS Handling",
    category: "APIs",
    description: "Configure CORS headers and handle preflight requests",
    features: ["CORS headers", "Preflight handling", "Origin validation"],
    apis: ["Headers", "Response"],
    difficulty: "intermediate",
    path: "/cors",
  },
  {
    id: "form-handling",
    name: "Form Handling",
    category: "Web Features",
    description: "Process HTML forms with validation and response handling",
    features: ["Form parsing", "Validation", "FormData"],
    apis: ["FormData", "Request"],
    difficulty: "beginner",
    path: "/form-handling",
  },
  {
    id: "security-headers",
    name: "Security Headers",
    category: "Security",
    description: "Set important security headers (CSP, X-Frame-Options, etc)",
    features: ["CSP", "X-Frame-Options", "HSTS", "Referrer policy"],
    apis: ["Headers", "Response"],
    difficulty: "intermediate",
    path: "/security-headers",
  },
  {
    id: "rate-limiting",
    name: "Rate Limiting",
    category: "Infrastructure",
    description: "Implement request rate limiting with time windows",
    features: ["In-memory limiting", "Rate limits", "Client tracking"],
    apis: ["Map", "setTimeout"],
    difficulty: "intermediate",
    path: "/rate-limiting",
  },
  {
    id: "request-modification",
    name: "Request Modification",
    category: "Routing",
    description: "Modify request/response headers and bodies",
    features: ["Header manipulation", "Response transformation", "Content negotiation"],
    apis: ["Headers", "Request", "Response"],
    difficulty: "intermediate",
    path: "/request-modification",
  },
  {
    id: "caching",
    name: "Response Caching",
    category: "Infrastructure",
    description: "Cache responses with TTL and cache headers",
    features: ["In-memory cache", "TTL", "Cache control"],
    apis: ["Map", "Cache headers"],
    difficulty: "intermediate",
    path: "/caching",
  },
  {
    id: "error-handling",
    name: "Error Handling",
    category: "Best Practices",
    description: "Structured error handling with logging and tracing",
    features: ["Error responses", "Request logging", "Unique IDs"],
    apis: ["Error handling", "JSON"],
    difficulty: "beginner",
    path: "/error-handling",
  },
  {
    id: "middleware",
    name: "Middleware Pattern",
    category: "Architecture",
    description: "Implement composable middleware for request processing",
    features: ["Middleware composition", "Chain of responsibility", "Auth/CORS/Logging"],
    apis: ["Function composition", "Async"],
    difficulty: "advanced",
    path: "/middleware",
  },
  {
    id: "data-processing",
    name: "Data Processing",
    category: "Data",
    description: "Parse, transform, and analyze data (CSV↔JSON)",
    features: ["CSV parsing", "Data transformation", "Statistics"],
    apis: ["Array methods", "String methods"],
    difficulty: "intermediate",
    path: "/data-processing",
  },
  {
    id: "urlpattern",
    name: "URLPattern Routing",
    category: "Routing",
    description: "Advanced routing with URLPattern API for complex URLs",
    features: ["URLPattern", "Named parameters", "Wildcard routes"],
    apis: ["URLPattern", "URL"],
    difficulty: "intermediate",
    path: "/urlpattern-routing",
  },
  {
    id: "compression",
    name: "Compression Streams",
    category: "Streams",
    description: "Compress/decompress data using gzip and deflate",
    features: ["CompressionStream", "DecompressionStream", "Web Streams"],
    apis: ["CompressionStream", "ReadableStream"],
    difficulty: "intermediate",
    path: "/compression-stream",
  },
  {
    id: "crypto",
    name: "Web Crypto API",
    category: "Security",
    description: "Hashing, encryption, UUID generation, and digital signatures",
    features: ["AES-GCM encryption", "SHA hashing", "UUID generation"],
    apis: ["SubtleCrypto", "crypto.getRandomValues"],
    difficulty: "advanced",
    path: "/web-crypto-api",
  },
  {
    id: "transform",
    name: "Transform Streams",
    category: "Streams",
    description: "Pipeline data through multiple transformations",
    features: ["TransformStream", "Stream composition", "Data pipelines"],
    apis: ["TransformStream", "ReadableStream"],
    difficulty: "advanced",
    path: "/transform-stream",
  },
  {
    id: "intl",
    name: "Intl API",
    category: "Internationalization",
    description: "Format dates, numbers, and text for different locales",
    features: ["DateTimeFormat", "NumberFormat", "Collation", "Relative time"],
    apis: ["Intl.DateTimeFormat", "Intl.NumberFormat", "Intl.Collator"],
    difficulty: "intermediate",
    path: "/intl-api",
  },
  {
    id: "sse",
    name: "Server-Sent Events",
    category: "Networking",
    description: "Real-time server-to-client communication using SSE",
    features: ["ReadableStream", "Event streaming", "Real-time updates"],
    apis: ["ReadableStream", "Response"],
    difficulty: "intermediate",
    path: "/server-sent-events",
  },
  {
    id: "abort",
    name: "AbortController",
    category: "Advanced",
    description: "Cancel requests and operations using AbortSignal",
    features: ["Request cancellation", "Timeouts", "Abort handling"],
    apis: ["AbortController", "AbortSignal"],
    difficulty: "intermediate",
    path: "/abort-controller",
  },
  {
    id: "performance",
    name: "Performance API",
    category: "Profiling",
    description: "Profile code and benchmark operations for performance",
    features: ["Benchmarking", "Performance measurement", "Metrics"],
    apis: ["performance.now", "performance.mark/measure"],
    difficulty: "intermediate",
    path: "/performance-api",
  },
  {
    id: "generators",
    name: "Generators",
    category: "Advanced",
    description: "Memory-efficient lazy data processing with generators",
    features: ["Generator functions", "Lazy evaluation", "Pagination"],
    apis: ["function*", "yield", "for...of"],
    difficulty: "advanced",
    path: "/generators",
  },
];

Deno.serve((req) => {
  const url = new URL(req.url);

  // Serve the HTML page
  if (url.pathname === "/" || url.pathname === "/index") {
    const html = generateIndexPage();
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }

  // API: Get all examples
  if (url.pathname === "/api/examples" && req.method === "GET") {
    const category = url.searchParams.get("category");
    const difficulty = url.searchParams.get("difficulty");
    const search = url.searchParams.get("search")?.toLowerCase();

    let filtered = examples;

    if (category) {
      filtered = filtered.filter((e) => e.category === category);
    }

    if (difficulty) {
      filtered = filtered.filter((e) => e.difficulty === difficulty);
    }

    if (search) {
      filtered = filtered.filter(
        (e) =>
          e.name.toLowerCase().includes(search) ||
          e.description.toLowerCase().includes(search) ||
          e.features.some((f) => f.toLowerCase().includes(search))
      );
    }

    return new Response(JSON.stringify(filtered, null, 2), {
      headers: { "content-type": "application/json" },
    });
  }

  // API: Get categories
  if (url.pathname === "/api/categories" && req.method === "GET") {
    const categories = Array.from(
      new Set(examples.map((e) => e.category))
    ).sort();

    return new Response(JSON.stringify(categories, null, 2), {
      headers: { "content-type": "application/json" },
    });
  }

  // API: Get example by ID
  if (url.pathname.startsWith("/api/examples/")) {
    const id = url.pathname.replace("/api/examples/", "");
    const example = examples.find((e) => e.id === id);

    if (example) {
      return new Response(JSON.stringify(example, null, 2), {
        headers: { "content-type": "application/json" },
      });
    }

    return new Response(JSON.stringify({ error: "Not found" }), {
      status: 404,
      headers: { "content-type": "application/json" },
    });
  }

  return new Response("Not found", { status: 404 });
});

function generateIndexPage(): string {
  const categories = Array.from(
    new Set(examples.map((e) => e.category))
  ).sort();

  const html = `
    <!DOCTYPE html>
    <html>
    <head>
      <meta charset="UTF-8">
      <meta name="viewport" content="width=device-width, initial-scale=1.0">
      <title>Deno Edge Runtime - Examples & Documentation</title>
      <style>
        * {
          margin: 0;
          padding: 0;
          box-sizing: border-box;
        }

        :root {
          --primary: #667eea;
          --secondary: #764ba2;
          --background: #f5f5f5;
          --surface: #ffffff;
          --text-primary: #333333;
          --text-secondary: #666666;
          --border: #e0e0e0;
          --success: #4CAF50;
          --warning: #ff9800;
          --error: #f44336;
        }

        body {
          font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
          background: var(--background);
          color: var(--text-primary);
          line-height: 1.6;
        }

        /* Header */
        .header {
          background: linear-gradient(135deg, var(--primary) 0%, var(--secondary) 100%);
          color: white;
          padding: 60px 20px;
          text-align: center;
          box-shadow: 0 4px 20px rgba(0, 0, 0, 0.1);
        }

        .header h1 {
          font-size: 3em;
          margin-bottom: 10px;
          font-weight: 700;
        }

        .header p {
          font-size: 1.2em;
          opacity: 0.95;
          margin-bottom: 20px;
        }

        .header-badges {
          display: flex;
          gap: 10px;
          justify-content: center;
          flex-wrap: wrap;
        }

        .badge {
          background: rgba(255, 255, 255, 0.2);
          padding: 8px 16px;
          border-radius: 20px;
          font-size: 0.9em;
          border: 1px solid rgba(255, 255, 255, 0.3);
        }

        /* Controls */
        .controls {
          background: var(--surface);
          padding: 20px;
          margin: 30px auto;
          max-width: 1200px;
          border-radius: 8px;
          box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
        }

        .control-row {
          display: grid;
          grid-template-columns: 1fr 1fr 1fr auto;
          gap: 15px;
          align-items: center;
        }

        .control-group {
          display: flex;
          flex-direction: column;
          gap: 5px;
        }

        .control-group label {
          font-weight: 600;
          font-size: 0.9em;
          color: var(--text-secondary);
        }

        input[type="text"],
        select {
          padding: 12px;
          border: 1px solid var(--border);
          border-radius: 4px;
          font-size: 1em;
          font-family: inherit;
          background: var(--surface);
          color: var(--text-primary);
        }

        input[type="text"]:focus,
        select:focus {
          outline: none;
          border-color: var(--primary);
          box-shadow: 0 0 0 3px rgba(102, 126, 234, 0.1);
        }

        button {
          background: var(--primary);
          color: white;
          border: none;
          padding: 12px 24px;
          border-radius: 4px;
          cursor: pointer;
          font-size: 1em;
          font-weight: 600;
          transition: all 0.3s;
        }

        button:hover {
          background: var(--secondary);
          transform: translateY(-2px);
          box-shadow: 0 4px 12px rgba(102, 126, 234, 0.4);
        }

        button:active {
          transform: translateY(0);
        }

        /* Grid */
        .container {
          max-width: 1200px;
          margin: 0 auto;
          padding: 0 20px 40px;
        }

        .examples-grid {
          display: grid;
          grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
          gap: 20px;
          margin-top: 30px;
        }

        /* Card */
        .example-card {
          background: var(--surface);
          border-radius: 12px;
          padding: 24px;
          box-shadow: 0 2px 12px rgba(0, 0, 0, 0.08);
          transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);
          cursor: pointer;
          border: 2px solid transparent;
        }

        .example-card:hover {
          transform: translateY(-8px);
          box-shadow: 0 12px 32px rgba(102, 126, 234, 0.2);
          border-color: var(--primary);
        }

        .example-card-header {
          display: flex;
          justify-content: space-between;
          align-items: start;
          margin-bottom: 12px;
        }

        .example-card h3 {
          font-size: 1.3em;
          color: var(--text-primary);
          flex: 1;
        }

        .difficulty-badge {
          padding: 4px 12px;
          border-radius: 12px;
          font-size: 0.75em;
          font-weight: 700;
          text-transform: uppercase;
          letter-spacing: 0.5px;
        }

        .difficulty-beginner {
          background: #d4edda;
          color: #155724;
        }

        .difficulty-intermediate {
          background: #fff3cd;
          color: #856404;
        }

        .difficulty-advanced {
          background: #f8d7da;
          color: #721c24;
        }

        .example-category {
          display: inline-block;
          background: rgba(102, 126, 234, 0.1);
          color: var(--primary);
          padding: 4px 12px;
          border-radius: 4px;
          font-size: 0.85em;
          font-weight: 600;
          margin-bottom: 12px;
        }

        .example-description {
          color: var(--text-secondary);
          margin-bottom: 16px;
          line-height: 1.5;
        }

        .example-features {
          margin-bottom: 16px;
        }

        .feature-list {
          display: flex;
          flex-wrap: wrap;
          gap: 8px;
        }

        .feature-tag {
          background: #f0f0f0;
          color: var(--text-primary);
          padding: 4px 10px;
          border-radius: 4px;
          font-size: 0.8em;
          border-left: 3px solid var(--primary);
        }

        .example-apis {
          margin-bottom: 16px;
          padding-top: 16px;
          border-top: 1px solid var(--border);
        }

        .api-list {
          display: flex;
          flex-wrap: wrap;
          gap: 8px;
        }

        .api-tag {
          background: var(--primary);
          color: white;
          padding: 4px 10px;
          border-radius: 4px;
          font-size: 0.75em;
          font-weight: 600;
        }

        .example-link {
          display: inline-block;
          background: var(--primary);
          color: white;
          padding: 10px 20px;
          border-radius: 4px;
          text-decoration: none;
          font-weight: 600;
          transition: all 0.3s;
          margin-top: 12px;
        }

        .example-link:hover {
          background: var(--secondary);
          transform: translateX(4px);
        }

        /* Stats */
        .stats {
          display: grid;
          grid-template-columns: repeat(4, 1fr);
          gap: 20px;
          margin: 40px auto;
          max-width: 1200px;
        }

        .stat-card {
          background: var(--surface);
          padding: 30px 20px;
          border-radius: 12px;
          text-align: center;
          box-shadow: 0 2px 12px rgba(0, 0, 0, 0.08);
        }

        .stat-value {
          font-size: 2.5em;
          font-weight: 700;
          color: var(--primary);
          margin-bottom: 8px;
        }

        .stat-label {
          color: var(--text-secondary);
          font-size: 0.95em;
        }

        /* Responsive */
        @media (max-width: 768px) {
          .header h1 {
            font-size: 2em;
          }

          .control-row {
            grid-template-columns: 1fr;
          }

          .examples-grid {
            grid-template-columns: 1fr;
          }

          .stats {
            grid-template-columns: repeat(2, 1fr);
          }

          .stat-card {
            padding: 20px 15px;
          }

          .stat-value {
            font-size: 2em;
          }
        }

        /* Loading */
        .loading {
          text-align: center;
          padding: 40px;
          color: var(--text-secondary);
        }

        .spinner {
          display: inline-block;
          width: 40px;
          height: 40px;
          border: 4px solid var(--border);
          border-top-color: var(--primary);
          border-radius: 50%;
          animation: spin 1s linear infinite;
        }

        @keyframes spin {
          to { transform: rotate(360deg); }
        }

        /* Footer */
        .footer {
          background: var(--text-primary);
          color: white;
          padding: 40px 20px;
          text-align: center;
          margin-top: 60px;
        }

        .footer a {
          color: var(--primary);
          text-decoration: none;
        }

        .footer a:hover {
          text-decoration: underline;
        }

        /* No results */
        .no-results {
          text-align: center;
          padding: 60px 20px;
          color: var(--text-secondary);
        }

        .no-results svg {
          width: 80px;
          height: 80px;
          opacity: 0.3;
          margin-bottom: 20px;
        }
      </style>
    </head>
    <body>
      <!-- Header -->
      <div class="header">
        <h1>🚀 Deno Edge Runtime</h1>
        <p>Complete Examples & Documentation Library</p>
        <div class="header-badges">
          <div class="badge">27 Examples</div>
          <div class="badge">Web Standards APIs</div>
          <div class="badge">Production Ready</div>
        </div>
      </div>

      <!-- Stats -->
      <div class="stats">
        <div class="stat-card">
          <div class="stat-value">${examples.length}</div>
          <div class="stat-label">Examples</div>
        </div>
        <div class="stat-card">
          <div class="stat-value">${new Set(examples.map((e) => e.category)).size}</div>
          <div class="stat-label">Categories</div>
        </div>
        <div class="stat-card">
          <div class="stat-value">${examples.filter((e) => e.difficulty === "advanced").length}</div>
          <div class="stat-label">Advanced</div>
        </div>
        <div class="stat-card">
          <div class="stat-value">100%</div>
          <div class="stat-label">Web Standards</div>
        </div>
      </div>

      <!-- Controls -->
      <div class="controls">
        <div class="control-row">
          <div class="control-group">
            <label for="searchInput">Search</label>
            <input type="text" id="searchInput" placeholder="Search examples...">
          </div>
          <div class="control-group">
            <label for="categoryFilter">Category</label>
            <select id="categoryFilter">
              <option value="">All Categories</option>
              ${categories.map((cat) => `<option value="${cat}">${cat}</option>`).join("")}
            </select>
          </div>
          <div class="control-group">
            <label for="difficultyFilter">Difficulty</label>
            <select id="difficultyFilter">
              <option value="">All Levels</option>
              <option value="beginner">Beginner</option>
              <option value="intermediate">Intermediate</option>
              <option value="advanced">Advanced</option>
            </select>
          </div>
          <button onclick="loadExamples()">Search</button>
        </div>
      </div>

      <!-- Container -->
      <div class="container">
        <div id="examplesGrid" class="examples-grid">
          <div class="loading">
            <div class="spinner"></div>
            <p>Loading examples...</p>
          </div>
        </div>
      </div>

      <!-- Footer -->
      <div class="footer">
        <p>Made with ❤️ for Deno Edge Runtime</p>
        <p style="margin-top: 10px; font-size: 0.9em; opacity: 0.8;">
          All examples use Web Standards APIs with zero external dependencies
        </p>
      </div>

      <script>
        async function loadExamples() {
          const search = document.getElementById('searchInput').value;
          const category = document.getElementById('categoryFilter').value;
          const difficulty = document.getElementById('difficultyFilter').value;

          const params = new URLSearchParams();
          if (search) params.append('search', search);
          if (category) params.append('category', category);
          if (difficulty) params.append('difficulty', difficulty);

          try {
            const response = await fetch('./api/examples?' + params.toString());

            if (!response.ok) {
              throw new Error(\`Failed to load examples: \${response.status} \${response.statusText}\`);
            }

            const data = await response.json();

            if (!Array.isArray(data)) {
              throw new Error('Invalid response format: expected an array');
            }

            if (data.length === 0) {
              document.getElementById('examplesGrid').innerHTML = \`
                <div class="no-results" style="grid-column: 1 / -1;">
                  <h2>No examples found</h2>
                  <p>Try adjusting your search criteria</p>
                </div>
              \`;
              return;
            }

            document.getElementById('examplesGrid').innerHTML = data
              .map(renderCard)
              .join('');
          } catch (error) {
            document.getElementById('examplesGrid').innerHTML = \`
              <div class="no-results" style="grid-column: 1 / -1;">
                <h2>Error loading examples</h2>
                <p>\${error.message}</p>
              </div>
            \`;
          }
        }

        function renderCard(example) {
          const difficultyClass = \`difficulty-\${example.difficulty}\`;
          return \`
            <div class="example-card">
              <div class="example-card-header">
                <h3>\${example.name}</h3>
                <span class="difficulty-badge \${difficultyClass}">\${example.difficulty}</span>
              </div>

              <span class="example-category">\${example.category}</span>

              <p class="example-description">\${example.description}</p>

              <div class="example-features">
                <div class="feature-list">
                  \${example.features.map((f) => \`<span class="feature-tag">✓ \${f}</span>\`).join("")}
                </div>
              </div>

              <div class="example-apis">
                <div class="api-list">
                  \${example.apis.map((a) => \`<span class="api-tag">\${a}</span>\`).join("")}
                </div>
              </div>

              <a href="\${example.path}" class="example-link">View Example →</a>
            </div>
          \`;
        }

        // Event listeners
        document.getElementById('searchInput').addEventListener('keyup', (e) => {
          if (e.key === 'Enter') loadExamples();
        });

        document.getElementById('categoryFilter').addEventListener('change', loadExamples);
        document.getElementById('difficultyFilter').addEventListener('change', loadExamples);

        // Initial load
        loadExamples();
      </script>
    </body>
    </html>
  `;

  return html;
}
