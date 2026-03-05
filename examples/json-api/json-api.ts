// Example: JSON API
// Simple REST API that returns JSON data

const routes = {
  "/api/users": {
    GET: () => [
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Charlie", email: "charlie@example.com" },
    ],
  },
  "/api/users/:id": {
    GET: (id: string) => ({
      id: parseInt(id),
      name: `User ${id}`,
      email: `user${id}@example.com`,
    }),
  },
  "/api/echo": {
    POST: async (body: unknown) => ({
      message: "Echo received",
      data: body,
      timestamp: new Date().toISOString(),
    }),
  },
};

Deno.serve(async (req) => {
  const url = new URL(req.url);
  const method = req.method;
  const pathname = url.pathname;

  // Handle CORS preflight
  if (method === "OPTIONS") {
    return new Response(null, {
      headers: {
        "access-control-allow-origin": "*",
        "access-control-allow-methods": "GET, POST, OPTIONS",
        "access-control-allow-headers": "content-type",
      },
    });
  }

  // Route: GET /api/users
  if (pathname === "/api/users" && method === "GET") {
    const users = routes["/api/users"].GET();
    return new Response(JSON.stringify(users), {
      headers: {
        "content-type": "application/json",
        "access-control-allow-origin": "*",
      },
    });
  }

  // Route: GET /api/users/:id
  const userMatch = pathname.match(/^\/api\/users\/(\d+)$/);
  if (userMatch && method === "GET") {
    const user = routes["/api/users/:id"].GET(userMatch[1]);
    return new Response(JSON.stringify(user), {
      headers: {
        "content-type": "application/json",
        "access-control-allow-origin": "*",
      },
    });
  }

  // Route: POST /api/echo
  if (pathname === "/api/echo" && method === "POST") {
    try {
      const body = await req.json();
      const response = await routes["/api/echo"].POST(body);
      return new Response(JSON.stringify(response), {
        headers: {
          "content-type": "application/json",
          "access-control-allow-origin": "*",
        },
      });
    } catch {
      return new Response(JSON.stringify({ error: "Invalid JSON" }), {
        status: 400,
        headers: {
          "content-type": "application/json",
          "access-control-allow-origin": "*",
        },
      });
    }
  }

  // 404 Not Found
  return new Response(JSON.stringify({ error: "Not found" }), {
    status: 404,
    headers: {
      "content-type": "application/json",
      "access-control-allow-origin": "*",
    },
  });
});
