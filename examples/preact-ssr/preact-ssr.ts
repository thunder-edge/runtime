// Example: Preact with Server-Side Rendering (SSR)
// This example shows how to render Preact components on the server

// Note: This is a simplified example without external dependencies
// In production, you would use preact/compat or preact/render-to-string

// Simple HTML renderer for components
interface ComponentProps {
  [key: string]: unknown;
}

interface Component {
  (props: ComponentProps): string;
}

// Simple Button component
const Button: Component = (props) => {
  const { label, onClick } = props;
  return `<button onclick="${onClick || ""}"">${label || "Click me"}</button>`;
};

// Simple Card component
const Card: Component = (props) => {
  const { title, content } = props;
  return `
    <div class="card">
      <h2>${title}</h2>
      <p>${content}</p>
    </div>
  `;
};

// Layout component
const Layout: Component = (props) => {
  const { children, title } = props;
  return `
    <!DOCTYPE html>
    <html lang="en">
    <head>
      <meta charset="UTF-8">
      <meta name="viewport" content="width=device-width, initial-scale=1.0">
      <title>${title}</title>
      <style>
        * {
          margin: 0;
          padding: 0;
          box-sizing: border-box;
        }

        body {
          font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
          background: #f5f5f5;
          padding: 20px;
        }

        .container {
          max-width: 800px;
          margin: 0 auto;
        }

        h1 {
          color: #333;
          margin-bottom: 30px;
          text-align: center;
        }

        .card {
          background: white;
          border-radius: 8px;
          padding: 20px;
          margin-bottom: 20px;
          box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
        }

        .card h2 {
          color: #667eea;
          margin-bottom: 10px;
          font-size: 1.3em;
        }

        .card p {
          color: #666;
          line-height: 1.6;
        }

        button {
          background: #667eea;
          color: white;
          border: none;
          padding: 12px 24px;
          border-radius: 4px;
          cursor: pointer;
          font-size: 1em;
          transition: background 0.3s;
        }

        button:hover {
          background: #764ba2;
        }

        .grid {
          display: grid;
          grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
          gap: 20px;
        }

        .stats {
          display: flex;
          justify-content: space-around;
          margin: 20px 0;
        }

        .stat {
          background: white;
          padding: 20px;
          border-radius: 8px;
          text-align: center;
          flex: 1;
          margin: 0 10px;
        }

        .stat-value {
          font-size: 2em;
          color: #667eea;
          font-weight: bold;
        }

        .stat-label {
          color: #666;
          margin-top: 5px;
        }
      </style>
    </head>
    <body>
      <div class="container">
        <h1>${title}</h1>
        ${children}
      </div>

      <script>
        // Simple client-side interactivity
        console.log('Page loaded at', new Date().toISOString());
      </script>
    </body>
    </html>
  `;
};

// HomePage component
const HomePage: Component = () => {
  const cards = `
    <div class="grid">
      ${Card({ title: "Welcome", content: "This is a server-rendered Preact component running on edge!" })}
      ${Card({
        title: "Fast",
        content: "Server-side rendering ensures quick initial load times.",
      })}
      ${Card({ title: "Scalable", content: "Benefits from edge network infrastructure." })}
    </div>

    <div class="stats">
      <div class="stat">
        <div class="stat-value">99.9%</div>
        <div class="stat-label">Uptime</div>
      </div>
      <div class="stat">
        <div class="stat-value">&lt;50ms</div>
        <div class="stat-label">Latency</div>
      </div>
      <div class="stat">
        <div class="stat-value">∞</div>
        <div class="stat-label">Scalability</div>
      </div>
    </div>

    <div class="card" style="text-align: center;">
      ${Button({
        label: "Learn More",
        onClick: "alert('This is a button click!')",
      })}
    </div>
  `;

  return Layout({
    title: "Preact SSR Example",
    children: cards,
  });
};

// Main entry point
Deno.serve((req) => {
  const url = new URL(req.url);

  if (url.pathname === "/") {
    const html = HomePage({});
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }

  if (url.pathname === "/api/data") {
    return new Response(
      JSON.stringify({
        items: [
          { id: 1, title: "Item 1", value: 100 },
          { id: 2, title: "Item 2", value: 250 },
          { id: 3, title: "Item 3", value: 180 },
        ],
        timestamp: new Date().toISOString(),
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  return new Response("Not found", { status: 404 });
});
