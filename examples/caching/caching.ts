// Example: Simple Caching
// Demonstrates response caching strategies

// Simple in-memory cache
class SimpleCache {
  private cache = new Map<string, { data: string; expires: number }>();

  set(key: string, data: string, ttlSeconds: number = 60) {
    this.cache.set(key, {
      data,
      expires: Date.now() + ttlSeconds * 1000,
    });
  }

  get(key: string): string | null {
    const item = this.cache.get(key);
    if (!item) return null;

    if (Date.now() > item.expires) {
      this.cache.delete(key);
      return null;
    }

    return item.data;
  }

  clear(): void {
    this.cache.clear();
  }
}

const cache = new SimpleCache();

// Simulate an expensive operation
async function fetchExpensiveData(id: string): Promise<string> {
  // Simulate API call or heavy computation
  await new Promise((resolve) => setTimeout(resolve, 1000));
  return JSON.stringify({
    id,
    data: `Expensive data for ${id}`,
    generatedAt: new Date().toISOString(),
  });
}

Deno.serve(async (req) => {
  const url = new URL(req.url);

  // Home page with cache info
  if (url.pathname === "/") {
    const html = `
      <!DOCTYPE html>
      <html>
      <head>
        <title>Caching Example</title>
        <style>
          body { font-family: Arial; padding: 40px; background: #f5f5f5; }
          .container { max-width: 800px; margin: 0 auto; background: white; padding: 30px; border-radius: 8px; }
          h1 { color: #333; }
          button { background: #667eea; color: white; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; margin: 5px; }
          button:hover { background: #764ba2; }
          #output { background: #f9f9f9; padding: 20px; margin-top: 20px; border-radius: 4px; white-space: pre-wrap; font-family: monospace; }
          .timing { color: #999; margin-top: 10px; }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>💾 Response Caching</h1>
          <p>This example demonstrates caching of expensive operations.</p>

          <div>
            <button onclick="fetchData('item1', true)">Fetch with Cache (item1)</button>
            <button onclick="fetchData('item2', false)">Fetch without Cache (item2)</button>
            <button onclick="clearCache()">Clear Cache</button>
          </div>

          <div id="output"></div>
        </div>

        <script>
          async function fetchData(id, useCache) {
            const start = performance.now();
            const cacheParam = useCache ? 'true' : 'false';
            const response = await fetch(\`/api/data?id=\${id}&cache=\${cacheParam}\`);
            const data = await response.json();
            const duration = performance.now() - start;

            const output = \`
Request ID: \${id}
Cache Used: \${data.fromCache}
Duration: \${duration.toFixed(2)}ms
Data:
\${JSON.stringify(data, null, 2)}
            \`.trim();

            document.getElementById('output').textContent = output;
          }

          async function clearCache() {
            const response = await fetch('/api/cache/clear', { method: 'POST' });
            const data = await response.json();
            alert(data.message);
          }
        </script>
      </body>
      </html>
    `;
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }

  // API endpoint with caching
  if (url.pathname === "/api/data") {
    const id = url.searchParams.get("id") || "default";
    const useCache = url.searchParams.get("cache") === "true";

    const cacheKey = `data:${id}`;
    let data = null;
    let fromCache = false;

    if (useCache) {
      data = cache.get(cacheKey);
      if (data) {
        fromCache = true;
      }
    }

    if (!data) {
      data = await fetchExpensiveData(id);
      if (useCache) {
        cache.set(cacheKey, data, 60); // Cache for 60 seconds
      }
    }

    const response = {
      id,
      fromCache,
      data: JSON.parse(data),
      cacheEnabled: useCache,
    };

    return new Response(JSON.stringify(response, null, 2), {
      headers: {
        "content-type": "application/json",
        "cache-control": useCache ? "public, max-age=60" : "no-cache",
      },
    });
  }

  // Clear cache endpoint
  if (url.pathname === "/api/cache/clear" && req.method === "POST") {
    cache.clear();
    return new Response(
      JSON.stringify({
        message: "Cache cleared successfully",
        timestamp: new Date().toISOString(),
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  // Cache info endpoint
  if (url.pathname === "/api/cache/info") {
    return new Response(
      JSON.stringify({
        message: "Cache is working",
        ttl: "60 seconds (configurable)",
        strategy: "Time-based expiry",
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  return new Response("Not found", { status: 404 });
});
