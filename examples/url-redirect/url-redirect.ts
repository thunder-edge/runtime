// Example: URL Redirect and Rewriting
// Demonstrates URL redirects and path rewriting

Deno.serve((req) => {
  const url = new URL(req.url);
  const pathname = url.pathname;

  // Redirect rules
  const redirects: Record<string, string> = {
    "/old-page": "new-page",
    "/about-us": "about",
    "/contact-form": "contact",
    "/blog/2024": "blog?year=2024",
  };

  // Check for redirects
  if (pathname in redirects) {
    return new Response(null, {
      status: 301,
      headers: {
        location: redirects[pathname],
      },
    });
  }

  // URL rewriting (without changing the URL in browser)
  if (pathname.startsWith("/api/v1/")) {
    const path = pathname.replace("/api/v1/", "/api/");
    const html = `
      <!DOCTYPE html>
      <html>
      <head><title>URL Rewritten</title>
      <style>
        body { font-family: Arial; padding: 40px; background: #f5f5f5; }
        .box { background: white; padding: 30px; border-radius: 8px; max-width: 600px; margin: 0 auto; }
      </style>
      </head>
      <body>
        <div class="box">
          <h1>✓ URL Rewrite Successful</h1>
          <p><strong>Original URL:</strong> ${pathname}</p>
          <p><strong>Rewritten to:</strong> ${path}</p>
          <p>The request was internally rewritten without changing the browser URL.</p>
        </div>
      </body>
      </html>
    `;
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }

  // Country-based redirect (using Accept-Language as example)
  if (pathname === "/") {
    const lang = req.headers.get("accept-language") || "en";

    if (lang.startsWith("pt")) {
      return new Response(
        JSON.stringify({
          message: "Bem-vindo!",
          language: "Portuguese",
          url: req.url,
        }),
        {
          headers: { "content-type": "application/json" },
        }
      );
    }

    if (lang.startsWith("es")) {
      return new Response(
        JSON.stringify({
          message: "¡Bienvenido!",
          language: "Spanish",
          url: req.url,
        }),
        {
          headers: { "content-type": "application/json" },
        }
      );
    }

    return new Response(
      JSON.stringify({
        message: "Welcome!",
        language: "English",
        url: req.url,
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  // Default: show available redirects
  const html = `
    <!DOCTYPE html>
    <html>
    <head>
      <title>Redirect Examples</title>
      <style>
        body { font-family: Arial; padding: 40px; background: #f5f5f5; }
        .container { max-width: 600px; margin: 0 auto; background: white; padding: 30px; border-radius: 8px; }
        h1 { color: #333; }
        table { width: 100%; border-collapse: collapse; margin: 20px 0; }
        th, td { padding: 12px; text-align: left; border-bottom: 1px solid #ddd; }
        th { background: #667eea; color: white; }
        a { color: #667eea; text-decoration: none; }
        a:hover { text-decoration: underline; }
      </style>
    </head>
    <body>
      <div class="container">
        <h1>🔄 Redirect Examples</h1>
        <p>Try these redirect examples:</p>
        <table>
          <thead>
            <tr><th>From</th><th>To</th></tr>
          </thead>
          <tbody>
            <tr><td><a href="/old-page">/old-page</a></td><td>/new-page</td></tr>
            <tr><td><a href="/about-us">/about-us</a></td><td>/about</td></tr>
            <tr><td><a href="/contact-form">/contact-form</a></td><td>/contact</td></tr>
            <tr><td><a href="/blog/2024">/blog/2024</a></td><td>/blog?year=2024</td></tr>
          </tbody>
        </table>
        <h2>Language Detection</h2>
        <p>Visit <a href="/">/</a> with different Accept-Language headers to see language redirects.</p>
      </div>
    </body>
    </html>
  `;

  return new Response(html, {
    headers: { "content-type": "text/html; charset=utf-8" },
  });
});
