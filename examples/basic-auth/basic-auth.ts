// Example: Basic Authentication
// Demonstrates HTTP Basic Auth using HMAC for timing-safe comparison

Deno.serve(async (req) => {
  const url = new URL(req.url);

  // Credentials (in real apps, use a database)
  const VALID_USERNAME = "admin";
  const VALID_PASSWORD = "secret123";

  // Public routes that don't require auth
  const publicRoutes = ["/health", "/login"];
  if (publicRoutes.includes(url.pathname)) {
    if (url.pathname === "/health") {
      return new Response(JSON.stringify({ status: "ok" }), {
        headers: { "content-type": "application/json" },
      });
    }
    if (url.pathname === "/login") {
      const html = `
        <!DOCTYPE html>
        <html>
        <head>
          <title>Login</title>
          <style>
            body { font-family: Arial; display: flex; justify-content: center; align-items: center; height: 100vh; background: #f5f5f5; margin: 0; }
            .login { background: white; padding: 40px; border-radius: 8px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
            input { width: 100%; padding: 10px; margin: 10px 0; border: 1px solid #ddd; border-radius: 4px; }
            button { width: 100%; padding: 10px; background: #4CAF50; color: white; border: none; border-radius: 4px; cursor: pointer; }
            button:hover { background: #45a049; }
          </style>
        </head>
        <body>
          <div class="login">
            <h2>Login</h2>
            <form method="POST" action="/dashboard">
              <input type="text" name="username" placeholder="Username (admin)" required>
              <input type="password" name="password" placeholder="Password (secret123)" required>
              <button type="submit">Login</button>
            </form>
          </div>
        </body>
        </html>
      `;
      return new Response(html, {
        headers: { "content-type": "text/html" },
      });
    }
  }

  // Check Basic Auth header
  const authHeader = req.headers.get("authorization");
  if (!authHeader || !authHeader.startsWith("Basic ")) {
    return new Response(JSON.stringify({ error: "Unauthorized" }), {
      status: 401,
      headers: {
        "content-type": "application/json",
        "www-authenticate": 'Basic realm="Edge Function"',
      },
    });
  }

  try {
    // Decode Base64 credentials
    const credentials = atob(authHeader.slice(6));
    const [username, password] = credentials.split(":");

    // Timing-safe comparison using built-in crypto
    const usernameCorrect = await timingSafeEqual(
      username,
      VALID_USERNAME
    );
    const passwordCorrect = await timingSafeEqual(
      password,
      VALID_PASSWORD
    );

    if (!usernameCorrect || !passwordCorrect) {
      return new Response(JSON.stringify({ error: "Invalid credentials" }), {
        status: 401,
        headers: {
          "content-type": "application/json",
          "www-authenticate": 'Basic realm="Edge Function"',
        },
      });
    }

    // Authenticated
    return new Response(
      JSON.stringify({
        message: "Welcome!",
        user: username,
        timestamp: new Date().toISOString(),
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  } catch {
    return new Response(JSON.stringify({ error: "Invalid authorization" }), {
      status: 400,
      headers: { "content-type": "application/json" },
    });
  }
});

// Timing-safe string comparison
async function timingSafeEqual(
  a: string,
  b: string
): Promise<boolean> {
  const encoder = new TextEncoder();
  const aBytes = encoder.encode(a);
  const bBytes = encoder.encode(b);

  // Use WebCrypto for timing-safe comparison
  const key = await crypto.subtle.importKey(
    "raw",
    new Uint8Array(32),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"]
  );

  const sig1 = await crypto.subtle.sign("HMAC", key, aBytes);
  const sig2 = await crypto.subtle.sign("HMAC", key, bBytes);

  const view1 = new Uint8Array(sig1);
  const view2 = new Uint8Array(sig2);

  let result = 0;
  for (let i = 0; i < view1.length; i++) {
    result |= view1[i] ^ view2[i];
  }
  return result === 0;
}
