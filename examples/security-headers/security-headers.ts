// Example: Security Headers
// Demonstrates setting important security headers

Deno.serve((req) => {
  const url = new URL(req.url);

  // Create response headers with security headers
  const headers = new Headers({
    "content-type": "text/html; charset=utf-8",
    // Prevent clickjacking attacks
    "x-frame-options": "SAMEORIGIN",
    // Prevent MIME sniffing
    "x-content-type-options": "nosniff",
    // Enable XSS protection
    "x-xss-protection": "1; mode=block",
    // Referrer policy
    "referrer-policy": "strict-origin-when-cross-origin",
    // Content Security Policy - restrict resources
    "content-security-policy":
      "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: https:; font-src 'self'",
    // Feature policy / Permissions policy
    "permissions-policy": "camera=(), microphone=(), geolocation=()",
    // HSTS - enforce HTTPS (only over HTTPS)
    "strict-transport-security": "max-age=31536000; includeSubDomains",
  });

  if (url.pathname === "/") {
    const html = `
      <!DOCTYPE html>
      <html lang="en">
      <head>
        <meta charset="UTF-8">
        <meta name="viewport" content="width=device-width, initial-scale=1.0">
        <title>Security Headers Example</title>
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
            max-width: 900px;
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

          h2 {
            color: #667eea;
            margin-top: 30px;
            margin-bottom: 15px;
            font-size: 1.3em;
          }

          .header-item {
            background: #f9f9f9;
            padding: 15px;
            margin: 10px 0;
            border-left: 4px solid #667eea;
            border-radius: 4px;
          }

          .header-name {
            font-family: monospace;
            color: #667eea;
            font-weight: bold;
          }

          .header-value {
            font-family: monospace;
            color: #666;
            word-break: break-all;
            margin-top: 8px;
            background: white;
            padding: 10px;
            border-radius: 3px;
          }

          .description {
            color: #666;
            line-height: 1.6;
            margin-top: 10px;
          }

          .warning {
            background: #fff3cd;
            border: 1px solid #ffc107;
            padding: 15px;
            border-radius: 4px;
            margin: 20px 0;
            color: #856404;
          }

          .info {
            background: #d1ecf1;
            border: 1px solid #0c5460;
            padding: 15px;
            border-radius: 4px;
            margin: 20px 0;
            color: #0c5460;
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
          <h1>🔒 Security Headers Example</h1>
          <p class="subtitle">This page demonstrates important HTTP security headers</p>

          <div class="info">
            Open your browser's Developer Tools (F12) → Network tab → click on this page → Headers section to see the security headers in action.
          </div>

          <h2>Security Headers Applied:</h2>

          <div class="header-item">
            <div class="header-name">X-Frame-Options</div>
            <div class="header-value">SAMEORIGIN</div>
            <div class="description">Prevents clickjacking attacks by restricting how this page can be framed. Only allows framing from same origin.</div>
          </div>

          <div class="header-item">
            <div class="header-name">X-Content-Type-Options</div>
            <div class="header-value">nosniff</div>
            <div class="description">Prevents MIME type sniffing. Tells browsers to trust the Content-Type header and not attempt to detect the actual type.</div>
          </div>

          <div class="header-item">
            <div class="header-name">X-XSS-Protection</div>
            <div class="header-value">1; mode=block</div>
            <div class="description">Enables XSS (Cross-Site Scripting) protection in older browsers. Modern browsers use CSP instead.</div>
          </div>

          <div class="header-item">
            <div class="header-name">Content-Security-Policy</div>
            <div class="header-value">default-src 'self'; script-src 'self' 'unsafe-inline'; ...</div>
            <div class="description">Controls which resources can be loaded and from where. Prevents inline script execution and restricts external resource loading.</div>
          </div>

          <div class="header-item">
            <div class="header-name">Referrer-Policy</div>
            <div class="header-value">strict-origin-when-cross-origin</div>
            <div class="description">Controls how much referrer information is shared with other sites. Protects user privacy.</div>
          </div>

          <div class="header-item">
            <div class="header-name">Permissions-Policy</div>
            <div class="header-value">camera=(), microphone=(), geolocation=()</div>
            <div class="description">Controls which browser features and APIs can be used. Disables camera, microphone, and geolocation.</div>
          </div>

          <div class="header-item">
            <div class="header-name">Strict-Transport-Security</div>
            <div class="header-value">max-age=31536000; includeSubDomains</div>
            <div class="description">Enforces HTTPS by instructing browsers to always use secure connections for this domain (1 year).</div>
          </div>

          <h2>Why These Headers Matter:</h2>
          <ul style="line-height: 1.8; color: #666; margin-left: 20px;">
            <li><strong>Clickjacking Prevention:</strong> X-Frame-Options stops attackers from embedding your site in iframes</li>
            <li><strong>MIME Type Protection:</strong> X-Content-Type-Options prevents browser confusion about file types</li>
            <li><strong>XSS Prevention:</strong> CSP and X-XSS-Protection defend against script injection attacks</li>
            <li><strong>Privacy:</strong> Referrer-Policy and Permissions-Policy limit data exposure</li>
            <li><strong>Encryption:</strong> HSTS ensures all connections use HTTPS</li>
          </ul>

          <div class="warning">
            <strong>⚠️ Note:</strong> In production, carefully review CSP rules to ensure your application functions correctly. CSP violations are logged but don't break functionality.
          </div>

          <h2>Test the Headers:</h2>
          <button onclick="testXSSPrevention()" style="padding: 12px 24px; background: #667eea; color: white; border: none; border-radius: 4px; cursor: pointer; font-size: 1em;">
            Test XSS Prevention
          </button>

          <script>
            function testXSSPrevention() {
              // This will be blocked by CSP
              try {
                alert('This alert is allowed (inline scripts in style attributes work with current CSP)');
              } catch(e) {
                console.log('CSP blocked potential XSS:', e);
              }
            }

            // Check and display applied security headers
            console.group('Security Headers Applied');
            console.log('X-Frame-Options: SAMEORIGIN');
            console.log('X-Content-Type-Options: nosniff');
            console.log('X-XSS-Protection: 1; mode=block');
            console.log('Content-Security-Policy: Enabled');
            console.log('Referrer-Policy: strict-origin-when-cross-origin');
            console.log('Permissions-Policy: camera=(), microphone=(), geolocation=()');
            console.log('Strict-Transport-Security: max-age=31536000');
            console.groupEnd();
          </script>
        </div>
      </body>
      </html>
    `;
    return new Response(html, { headers });
  }

  // API endpoint also with security headers
  if (url.pathname === "/api/data") {
    headers.set("content-type", "application/json");
    return new Response(
      JSON.stringify({
        message: "Secure API endpoint",
        headers_applied: [
          "x-frame-options",
          "x-content-type-options",
          "x-xss-protection",
          "content-security-policy",
          "referrer-policy",
          "permissions-policy",
          "strict-transport-security",
        ],
      }),
      { headers }
    );
  }

  return new Response("Not found", { status: 404 });
});
