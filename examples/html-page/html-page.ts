// Example: HTML Page
// Returns a complete HTML page with styling

Deno.serve((req) => {
  const html = `
    <!DOCTYPE html>
    <html lang="en">
    <head>
      <meta charset="UTF-8">
      <meta name="viewport" content="width=device-width, initial-scale=1.0">
      <title>Edge Function Example</title>
      <style>
        * {
          margin: 0;
          padding: 0;
          box-sizing: border-box;
        }

        body {
          font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
          background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
          min-height: 100vh;
          display: flex;
          align-items: center;
          justify-content: center;
          padding: 20px;
        }

        .container {
          background: white;
          border-radius: 8px;
          box-shadow: 0 10px 40px rgba(0, 0, 0, 0.1);
          padding: 40px;
          max-width: 600px;
          text-align: center;
        }

        h1 {
          color: #333;
          margin-bottom: 20px;
          font-size: 2.5em;
        }

        p {
          color: #666;
          line-height: 1.6;
          margin-bottom: 20px;
          font-size: 1.1em;
        }

        .timestamp {
          color: #999;
          font-size: 0.9em;
          border-top: 1px solid #eee;
          padding-top: 20px;
          margin-top: 20px;
        }

        .button {
          display: inline-block;
          padding: 12px 30px;
          background: #667eea;
          color: white;
          text-decoration: none;
          border-radius: 4px;
          transition: background 0.3s;
          margin-top: 20px;
        }

        .button:hover {
          background: #764ba2;
        }
      </style>
    </head>
    <body>
      <div class="container">
        <h1>🚀 Edge Function</h1>
        <p>Welcome to your edge function running at the edge of the network!</p>
        <a href="/" class="button">Refresh Page</a>
        <div class="timestamp">
          <p>Served at: ${new Date().toISOString()}</p>
          <p>Request path: ${new URL(req.url).pathname}</p>
        </div>
      </div>
    </body>
    </html>
  `;

  return new Response(html, {
    headers: { "content-type": "text/html; charset=utf-8" },
  });
});
