// Example: WebAssembly
// Loads and executes WebAssembly in an edge function

// Simple Fibonacci calculator written in WAT (WebAssembly Text Format)
// This is a basic example - in production you'd compile from Rust, C, etc.

const wasmCode = new Uint8Array([
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x07, 0x01, 0x60,
  0x01, 0x7f, 0x01, 0x7f, 0x03, 0x02, 0x00, 0x00, 0x07, 0x0b, 0x01, 0x07,
  0x66, 0x69, 0x62, 0x6f, 0x6e, 0x61, 0x63, 0x63, 0x69, 0x00, 0x00, 0x0a,
  0x17, 0x01, 0x15, 0x01, 0x01, 0x7f, 0x42, 0x00, 0x21, 0x01, 0x42, 0x01,
  0x21, 0x02, 0x03, 0x40, 0x20, 0x00, 0x42, 0x01, 0x51, 0x04, 0x40, 0x0c,
  0x02, 0x0b, 0x20, 0x01, 0x20, 0x02, 0x7c, 0x21, 0x03, 0x20, 0x02, 0x21,
  0x01, 0x20, 0x03, 0x21, 0x02, 0x20, 0x00, 0x42, 0x01, 0x7d, 0x21, 0x00,
  0x0c, 0x00, 0x0b, 0x0b, 0x20, 0x02, 0x0b,
]);

// Initialize WAT (WebAssembly Text) module - Fibonacci function
// This is equivalent to:
// (func $fibonacci (param $n i64) (result i64)
//   (local $a i64)
//   (local $b i64)
//   (local $temp i64)
//   (local.set $a (i64.const 0))
//   (local.set $b (i64.const 1))
//   (block $break
//     (loop $continue
//       (i64.eq (local.get $n) (i64.const 1))
//       (br_if $break)
//       (local.set $temp (i64.add (local.get $a) (local.get $b)))
//       (local.set $a (local.get $b))
//       (local.set $b (local.get $temp))
//       (local.set $n (i64.sub (local.get $n) (i64.const 1)))
//       (br $continue)
//     )
//   )
//   (local.get $b)
// )

Deno.serve(async (req) => {
  const url = new URL(req.url);

  // Route to calculate Fibonacci
  if (url.pathname.startsWith("/calculate")) {
    const n = parseInt(url.searchParams.get("n") || "5");

    if (isNaN(n) || n < 0 || n > 100) {
      return new Response(
        JSON.stringify({
          error: "Invalid input. Please provide a number between 0 and 100.",
        }),
        {
          status: 400,
          headers: { "content-type": "application/json" },
        }
      );
    }

    try {
      // Instantiate WAT module
      const wasmModule = await WebAssembly.instantiate(wasmCode);
      const fibonacci = wasmModule.instance.exports.fibonacci as (
        n: number
      ) => number;

      // Calculate Fibonacci
      const result = fibonacci(n);

      return new Response(
        JSON.stringify({
          function: "fibonacci",
          input: n,
          result: result,
          timestamp: new Date().toISOString(),
        }),
        {
          headers: { "content-type": "application/json" },
        }
      );
    } catch (error) {
      return new Response(
        JSON.stringify({
          error: "Failed to calculate: " + (error as Error).message,
        }),
        {
          status: 500,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  // Home page with examples
  if (url.pathname === "/") {
    const html = `
      <!DOCTYPE html>
      <html>
      <head>
        <title>WebAssembly Example</title>
        <style>
          body {
            font-family: Arial, sans-serif;
            max-width: 800px;
            margin: 50px auto;
            padding: 20px;
            background: #f5f5f5;
          }
          .container {
            background: white;
            padding: 30px;
            border-radius: 8px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
          }
          h1 {
            color: #333;
          }
          .example {
            background: #f9f9f9;
            padding: 15px;
            margin: 15px 0;
            border-left: 4px solid #667eea;
            border-radius: 4px;
          }
          code {
            background: #f0f0f0;
            padding: 2px 6px;
            border-radius: 3px;
          }
          a {
            color: #667eea;
            text-decoration: none;
          }
          a:hover {
            text-decoration: underline;
          }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>🔧 WebAssembly Example</h1>
          <p>This edge function uses WebAssembly for high-performance computation.</p>

          <h2>Try it out:</h2>
          <div class="example">
            <p><code>/calculate?n=5</code> → <a href="/calculate?n=5">Fibonacci of 5</a></p>
          </div>
          <div class="example">
            <p><code>/calculate?n=10</code> → <a href="/calculate?n=10">Fibonacci of 10</a></p>
          </div>
          <div class="example">
            <p><code>/calculate?n=20</code> → <a href="/calculate?n=20">Fibonacci of 20</a></p>
          </div>

          <h2>How it works:</h2>
          <ol>
            <li>A WebAssembly module is compiled from WAT (WebAssembly Text format)</li>
            <li>The module exports a Fibonacci calculation function</li>
            <li>The function is called from JavaScript with the input value</li>
            <li>Results are returned as JSON</li>
          </ol>

          <p><strong>Note:</strong> WebAssembly provides performance benefits for CPU-intensive tasks, while JavaScript with edge benefits from lower latency.</p>
        </div>
      </body>
      </html>
    `;
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }

  return new Response("Not found", { status: 404 });
});
