// Example: Error Handling and Logging
// Demonstrates proper error handling and request logging

Deno.serve(async (req) => {
  const url = new URL(req.url);
  const requestId = crypto.randomUUID();

  // Simple logger
  const log = {
    info: (message: string, data?: unknown) => {
      console.log(`[${requestId}] INFO:`, message, data || "");
    },
    error: (message: string, error?: Error) => {
      console.error(`[${requestId}] ERROR:`, message, error?.message || "");
    },
    warn: (message: string, data?: unknown) => {
      console.warn(`[${requestId}] WARN:`, message, data || "");
    },
  };

  // Log incoming request
  log.info("Incoming request", {
    method: req.method,
    path: url.pathname,
    headers: Object.fromEntries(req.headers),
  });

  // Error handler wrapper
  const handleRoute = async (
    handler: () => Promise<Response>
  ): Promise<Response> => {
    try {
      const response = await handler();
      log.info("Request completed successfully", {
        status: response.status,
      });
      return response;
    } catch (error) {
      log.error("Request failed", error as Error);
      return errorResponse(
        500,
        "Internal Server Error",
        (error as Error)?.message,
        requestId
      );
    }
  };

  // Error response formatter
  const errorResponse = (
    status: number,
    message: string,
    details?: string,
    id?: string
  ) => {
    return new Response(
      JSON.stringify({
        error: {
          status,
          message,
          details,
          requestId: id,
          timestamp: new Date().toISOString(),
        },
      }),
      {
        status,
        headers: { "content-type": "application/json" },
      }
    );
  };

  // Home page
  if (url.pathname === "/") {
    return handleRoute(async () => {
      const html = `
        <!DOCTYPE html>
        <html>
        <head>
          <title>Error Handling & Logging</title>
          <style>
            body { font-family: Arial; padding: 40px; background: #f5f5f5; }
            .container { max-width: 800px; margin: 0 auto; background: white; padding: 30px; border-radius: 8px; }
            h1 { color: #333; }
            .example { background: #f9f9f9; padding: 15px; margin: 15px 0; border-left: 4px solid #667eea; }
            a { color: #667eea; text-decoration: none; cursor: pointer; }
            a:hover { text-decoration: underline; }
            code { background: #f0f0f0; padding: 2px 6px; border-radius: 3px; }
          </style>
        </head>
        <body>
          <div class="container">
            <h1>🔍 Error Handling & Logging</h1>
            <p>Test various error scenarios and logging:</p>

            <div class="example">
              <p><a href="/api/success">✓ Successful request</a></p>
            </div>

            <div class="example">
              <p><a href="/api/not-found">✗ 404 Not Found</a></p>
            </div>

            <div class="example">
              <p><a href="/api/bad-request">✗ 400 Bad Request</a></p>
            </div>

            <div class="example">
              <p><a href="/api/unauthorized">✗ 401 Unauthorized</a></p>
            </div>

            <div class="example">
              <p><a href="/api/error">✗ 500 Internal Error</a></p>
            </div>

            <div class="example">
              <p><a href="/api/validation?email=invalid">✗ Validation Error</a></p>
            </div>

            <h2>Features:</h2>
            <ul style="line-height: 1.8;">
              <li>Unique request IDs for tracing</li>
              <li>Structured error responses</li>
              <li>Request logging with context</li>
              <li>Proper HTTP status codes</li>
              <li>Error details in development mode</li>
            </ul>

            <h2>Check browser console for logs!</h2>
            <p>Each request is logged with its unique request ID for tracking.</p>
          </div>
        </body>
        </html>
      `;
      return new Response(html, {
        headers: { "content-type": "text/html; charset=utf-8" },
      });
    });
  }

  // Success response
  if (url.pathname === "/api/success") {
    return handleRoute(async () => {
      log.info("Processing successful request");
      return new Response(
        JSON.stringify({
          success: true,
          message: "Request processed successfully",
          requestId,
        }),
        {
          headers: { "content-type": "application/json" },
        }
      );
    });
  }

  // 404 Not Found
  if (url.pathname === "/api/not-found") {
    return handleRoute(async () => {
      log.warn("Resource not found");
      return errorResponse(404, "Resource Not Found", undefined, requestId);
    });
  }

  // 400 Bad Request
  if (url.pathname === "/api/bad-request") {
    return handleRoute(async () => {
      log.warn("Bad request - missing parameters");
      return errorResponse(
        400,
        "Bad Request",
        "Missing required parameters",
        requestId
      );
    });
  }

  // 401 Unauthorized
  if (url.pathname === "/api/unauthorized") {
    return handleRoute(async () => {
      log.warn("Unauthorized access attempt");
      return errorResponse(
        401,
        "Unauthorized",
        "Valid authentication required",
        requestId
      );
    });
  }

  // 500 Internal Server Error
  if (url.pathname === "/api/error") {
    return handleRoute(async () => {
      log.warn("Triggering intentional error for demonstration");
      throw new Error("Something went wrong in processing");
    });
  }

  // Validation error
  if (url.pathname === "/api/validation") {
    return handleRoute(async () => {
      const email = url.searchParams.get("email");

      // Simple email validation
      const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
      if (!email) {
        log.warn("Missing email parameter");
        return errorResponse(
          400,
          "Validation Error",
          "Email parameter is required",
          requestId
        );
      }

      if (!emailRegex.test(email)) {
        log.warn("Invalid email format", { email });
        return errorResponse(
          400,
          "Validation Error",
          "Invalid email format",
          requestId
        );
      }

      log.info("Email validation successful", { email });
      return new Response(
        JSON.stringify({
          success: true,
          message: "Email is valid",
          email,
          requestId,
        }),
        {
          headers: { "content-type": "application/json" },
        }
      );
    });
  }

  // Default 404
  return handleRoute(async () => {
    log.warn("Unknown route requested");
    return errorResponse(404, "Not Found", url.pathname, requestId);
  });
});
