// Example: Rate Limiting
// Demonstrates request rate limiting using in-memory storage

// Simple in-memory rate limiter
class RateLimiter {
  private limits = new Map<string, { count: number; resetTime: number }>();
  private maxRequests: number;
  private windowMs: number;

  constructor(maxRequests: number = 10, windowMs: number = 60000) {
    this.maxRequests = maxRequests;
    this.windowMs = windowMs;
  }

  isAllowed(key: string): boolean {
    const now = Date.now();
    const record = this.limits.get(key);

    // If no record or window expired, reset
    if (!record || now > record.resetTime) {
      this.limits.set(key, {
        count: 1,
        resetTime: now + this.windowMs,
      });
      return true;
    }

    // Increment counter
    record.count++;

    // Check if limit exceeded
    if (record.count > this.maxRequests) {
      return false;
    }

    return true;
  }

  getRemaining(key: string): number {
    const record = this.limits.get(key);
    if (!record || Date.now() > record.resetTime) {
      return this.maxRequests;
    }
    return Math.max(0, this.maxRequests - record.count);
  }

  getResetTime(key: string): number {
    const record = this.limits.get(key);
    if (!record) {
      return Date.now() + this.windowMs;
    }
    return record.resetTime;
  }

  // Cleanup old entries periodically
  cleanup() {
    const now = Date.now();
    for (const [key, record] of this.limits.entries()) {
      if (now > record.resetTime) {
        this.limits.delete(key);
      }
    }
  }
}

// Create rate limiters for different endpoints
const generalLimiter = new RateLimiter(10, 60000); // 10 requests per minute
const apiLimiter = new RateLimiter(5, 60000); // 5 requests per minute
const loginLimiter = new RateLimiter(3, 300000); // 3 requests per 5 minutes

// Lazy cleanup avoids keeping a global timer alive during module bootstrap.
let lastCleanupAt = 0;
function maybeCleanupRateLimiters() {
  const now = Date.now();
  if (now - lastCleanupAt < 60000) {
    return;
  }
  generalLimiter.cleanup();
  apiLimiter.cleanup();
  loginLimiter.cleanup();
  lastCleanupAt = now;
}

// Helper to get client IP
function getClientIP(req: Request): string {
  return req.headers.get("x-forwarded-for") || "unknown";
}

Deno.serve((req) => {
  maybeCleanupRateLimiters();

  const url = new URL(req.url);
  const clientIP = getClientIP(req);

  // Home page
  if (url.pathname === "/" && req.method === "GET") {
    const allowed = generalLimiter.isAllowed(clientIP);

    if (!allowed) {
      return new Response(
        JSON.stringify({
          error: "Rate limit exceeded",
          message: `Too many requests from ${clientIP}`,
          resetTime: new Date(generalLimiter.getResetTime(clientIP)),
        }),
        {
          status: 429,
          headers: {
            "content-type": "application/json",
            "retry-after": "60",
          },
        }
      );
    }

    const remaining = generalLimiter.getRemaining(clientIP);
    const resetTime = generalLimiter.getResetTime(clientIP);

    const html = `
      <!DOCTYPE html>
      <html>
      <head>
        <meta charset="UTF-8">
        <meta name="viewport" content="width=device-width, initial-scale=1.0">
        <title>Rate Limiting Example</title>
        <style>
          * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
          }

          body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #f5f5f5;
            padding: 40px 20px;
          }

          .container {
            max-width: 800px;
            margin: 0 auto;
            background: white;
            border-radius: 8px;
            padding: 40px;
            box-shadow: 0 2px 10px rgba(0, 0, 0, 0.1);
          }

          h1 {
            color: #333;
            margin-bottom: 10px;
          }

          .subtitle {
            color: #999;
            margin-bottom: 30px;
          }

          .quota {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 30px;
            border-radius: 8px;
            margin: 20px 0;
            text-align: center;
          }

          .quota-value {
            font-size: 3em;
            font-weight: bold;
            margin-bottom: 10px;
          }

          .quota-label {
            font-size: 1.1em;
            opacity: 0.9;
          }

          .info {
            background: #e8f4f8;
            border-left: 4px solid #4a90e2;
            padding: 20px;
            border-radius: 4px;
            margin: 20px 0;
          }

          .stats {
            display: grid;
            grid-template-columns: 1fr 1fr;
            gap: 20px;
            margin: 20px 0;
          }

          .stat {
            background: #f9f9f9;
            padding: 20px;
            border-radius: 4px;
            border-left: 4px solid #667eea;
          }

          .stat-label {
            color: #999;
            font-size: 0.9em;
          }

          .stat-value {
            font-size: 1.5em;
            color: #333;
            font-weight: bold;
            margin-top: 5px;
          }

          button {
            background: #667eea;
            color: white;
            border: none;
            padding: 12px 24px;
            border-radius: 4px;
            cursor: pointer;
            font-size: 1em;
            margin-top: 20px;
            transition: background 0.3s;
          }

          button:hover {
            background: #764ba2;
          }

          button:disabled {
            background: #ccc;
            cursor: not-allowed;
          }

          h2 {
            color: #333;
            margin-top: 30px;
            margin-bottom: 15px;
          }

          ul {
            margin-left: 20px;
            line-height: 1.8;
            color: #666;
          }

          code {
            background: #f0f0f0;
            padding: 2px 6px;
            border-radius: 3px;
            font-family: monospace;
          }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>⏱️ Rate Limiting Example</h1>
          <p class="subtitle">This endpoint limits requests to protect against abuse</p>

          <div class="quota">
            <div class="quota-value">${remaining}</div>
            <div class="quota-label">Requests remaining</div>
          </div>

          <div class="stats">
            <div class="stat">
              <div class="stat-label">Client IP</div>
              <div class="stat-value">${clientIP}</div>
            </div>
            <div class="stat">
              <div class="stat-label">Reset Time</div>
              <div class="stat-value">${new Date(resetTime).toLocaleTimeString()}</div>
            </div>
          </div>

          <div class="info">
            <strong>Current Limits:</strong>
            <ul style="margin: 10px 0 0 20px; list-style: none;">
              <li>• General endpoints: 10 requests/minute</li>
              <li>• API endpoints: 5 requests/minute</li>
              <li>• Login endpoint: 3 requests/5 minutes</li>
            </ul>
          </div>

          <h2>Test Rate Limiting:</h2>
          <button onclick="testRequest()" id="testBtn">Make Request to /api/test</button>

          <h2>How It Works:</h2>
          <ul>
            <li>Each client IP address has a separate rate limit quota</li>
            <li>When quota is exceeded, a 429 (Too Many Requests) response is returned</li>
            <li>The <code>Retry-After</code> header indicates when to retry</li>
            <li>Old entries are cleaned up periodically to free memory</li>
          </ul>

          <h2>Try These Endpoints:</h2>
          <ul>
            <li><code>GET /</code> - General endpoint (10 req/min)</li>
            <li><code>POST /api/test</code> - API endpoint (5 req/min)</li>
            <li><code>POST /login</code> - Login endpoint (3 req/5 min)</li>
          </ul>
        </div>

        <script>
          async function testRequest() {
            const btn = document.getElementById('testBtn');
            btn.disabled = true;

            try {
              const response = await fetch('/api/test', { method: 'POST' });
              const data = await response.json();

              if (response.ok) {
                alert(\`✓ Request successful!\\nRemaining: \${data.remaining}\`);
              } else if (response.status === 429) {
                alert(\`✗ Rate limit exceeded!\\nReset at: \${new Date(data.resetTime).toLocaleTimeString()}\`);
              }
            } catch (error) {
              alert('Error: ' + error.message);
            } finally {
              btn.disabled = false;
            }
          }
        </script>
      </body>
      </html>
    `;

    return new Response(html, {
      headers: {
        "content-type": "text/html; charset=utf-8",
        "x-ratelimit-limit": "10",
        "x-ratelimit-remaining": remaining.toString(),
        "x-ratelimit-reset": generalLimiter.getResetTime(clientIP).toString(),
      },
    });
  }

  // API endpoint with stricter rate limit
  if (url.pathname === "/api/test" && req.method === "POST") {
    const allowed = apiLimiter.isAllowed(clientIP);

    if (!allowed) {
      const resetTime = apiLimiter.getResetTime(clientIP);
      return new Response(
        JSON.stringify({
          error: "Rate limit exceeded",
          message: "API rate limit: 5 requests per minute",
          resetTime: new Date(resetTime),
        }),
        {
          status: 429,
          headers: {
            "content-type": "application/json",
            "retry-after": Math.ceil((resetTime - Date.now()) / 1000).toString(),
            "x-ratelimit-limit": "5",
            "x-ratelimit-remaining": "0",
            "x-ratelimit-reset": resetTime.toString(),
          },
        }
      );
    }

    const remaining = apiLimiter.getRemaining(clientIP);
    return new Response(
      JSON.stringify({
        success: true,
        message: "Request processed",
        remaining: remaining,
        resetTime: new Date(apiLimiter.getResetTime(clientIP)),
      }),
      {
        headers: {
          "content-type": "application/json",
          "x-ratelimit-limit": "5",
          "x-ratelimit-remaining": remaining.toString(),
          "x-ratelimit-reset": apiLimiter.getResetTime(clientIP).toString(),
        },
      }
    );
  }

  // Login endpoint with very strict rate limit
  if (url.pathname === "/login" && req.method === "POST") {
    const allowed = loginLimiter.isAllowed(clientIP);

    if (!allowed) {
      const resetTime = loginLimiter.getResetTime(clientIP);
      return new Response(
        JSON.stringify({
          error: "Too many login attempts",
          message: "Login rate limit: 3 attempts per 5 minutes",
          resetTime: new Date(resetTime),
        }),
        {
          status: 429,
          headers: {
            "content-type": "application/json",
            "retry-after": Math.ceil((resetTime - Date.now()) / 1000).toString(),
          },
        }
      );
    }

    const remaining = loginLimiter.getRemaining(clientIP);
    return new Response(
      JSON.stringify({
        message: "Login attempt processed",
        remaining: remaining,
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  return new Response("Not found", { status: 404 });
});
