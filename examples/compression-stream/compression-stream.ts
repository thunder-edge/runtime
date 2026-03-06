// Example: CompressionStream - Compress and Decompress Data
// Demonstrates gzip and deflate compression using Web Streams

Deno.serve(async (req) => {
  const url = new URL(req.url);

  // Helper to compress data with specified algorithm
  async function compressData(
    data: string,
    format: "gzip" | "deflate"
  ): Promise<Uint8Array> {
    const encoder = new TextEncoder();
    const input = encoder.encode(data);

    const compressed = ReadableStream.from([input]).pipeThrough(
      new CompressionStream(format)
    );

    const chunks: Uint8Array[] = [];
    const reader = compressed.getReader();
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(value instanceof Uint8Array ? value : new Uint8Array(value));
      }
    } finally {
      reader.releaseLock();
    }

    const total = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
    const merged = new Uint8Array(total);
    let offset = 0;
    for (const chunk of chunks) {
      merged.set(chunk, offset);
      offset += chunk.length;
    }
    return merged;
  }

  // Helper to decompress data
  async function decompressData(
    compressed: Uint8Array,
    format: "gzip" | "deflate"
  ): Promise<string> {
    const input = new Uint8Array(compressed);
    const decompressed = ReadableStream.from([input]).pipeThrough(
      new DecompressionStream(format)
    );

    const chunks: Uint8Array[] = [];
    const reader = decompressed.getReader();
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(value instanceof Uint8Array ? value : new Uint8Array(value));
      }
    } finally {
      reader.releaseLock();
    }

    const total = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
    const merged = new Uint8Array(total);
    let offset = 0;
    for (const chunk of chunks) {
      merged.set(chunk, offset);
      offset += chunk.length;
    }

    return new TextDecoder().decode(merged);
  }

  // Compress endpoint
  if (url.pathname === "/api/compress" && req.method === "POST") {
    try {
      const format = (url.searchParams.get("format") || "gzip") as
        | "gzip"
        | "deflate";
      const body = await req.text();

      const compressed = await compressData(body, format);
      const ratio = (
        ((1 - compressed.length / body.length) * 100)
      ).toFixed(2);

      return new Response(
        JSON.stringify({
          original_size: body.length,
          compressed_size: compressed.length,
          compression_ratio: ratio + "%",
          format: format,
          compressed_base64: btoa(String.fromCharCode(...compressed)),
        }, null, 2),
        {
          headers: { "content-type": "application/json" },
        }
      );
    } catch (error) {
      return new Response(
        JSON.stringify({
          error: "Compression failed",
          details: (error as Error)?.message,
        }),
        {
          status: 400,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  // Decompress endpoint
  if (url.pathname === "/api/decompress" && req.method === "POST") {
    try {
      const format = (url.searchParams.get("format") || "gzip") as
        | "gzip"
        | "deflate";
      const body = await req.json() as { compressed_base64: string };
      const compressed = new Uint8Array(
        atob(body.compressed_base64)
          .split("")
          .map((c: string) => c.charCodeAt(0))
      );

      const decompressed = await decompressData(compressed, format);

      return new Response(
        JSON.stringify({
          original_size: compressed.length,
          decompressed_size: decompressed.length,
          format: format,
          content: decompressed.substring(0, 200) + (decompressed.length > 200 ? "..." : ""),
        }, null, 2),
        {
          headers: { "content-type": "application/json" },
        }
      );
    } catch (error) {
      return new Response(
        JSON.stringify({
          error: "Decompression failed",
          details: (error as Error)?.message,
        }),
        {
          status: 400,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  // Home page
  if (url.pathname === "/") {
    const html = `
      <!DOCTYPE html>
      <html>
      <head>
        <meta charset="UTF-8">
        <title>CompressionStream</title>
        <style>
          * { margin: 0; padding: 0; box-sizing: border-box; }
          body { font-family: Arial; background: #f5f5f5; padding: 40px 20px; }
          .container { max-width: 900px; margin: 0 auto; background: white; border-radius: 8px; padding: 40px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
          h1 { color: #333; margin-bottom: 10px; }
          .example { background: #f9f9f9; border-left: 4px solid #667eea; padding: 20px; margin: 20px 0; border-radius: 4px; }
          textarea { width: 100%; padding: 12px; border: 1px solid #ddd; border-radius: 4px; font-family: monospace; font-size: 0.9em; }
          button { background: #667eea; color: white; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; margin: 10px 5px 10px 0; }
          button:hover { background: #764ba2; }
          .output { background: white; padding: 15px; border: 1px solid #ddd; border-radius: 4px; margin-top: 15px; max-height: 300px; overflow-y: auto; white-space: pre-wrap; font-family: monospace; font-size: 0.9em; }
          select { padding: 8px; border: 1px solid #ddd; border-radius: 4px; }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>🗜️ CompressionStream API</h1>
          <p style="color: #999; margin-bottom: 20px;">Compress and decompress data using gzip or deflate</p>

          <div class="example">
            <h2>Compression</h2>
            <p style="margin-bottom: 10px;">Format:
              <select id="compressFormat" onchange="localStorage.setItem('compressFormat', this.value)">
                <option value="gzip">gzip</option>
                <option value="deflate">deflate</option>
              </select>
            </p>
            <textarea id="compressInput" placeholder="Enter text to compress..." rows="6">Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.</textarea>
            <button onclick="compress()">Compress</button>
            <div id="compressOutput" class="output"></div>
          </div>

          <div class="example">
            <h2>Decompression</h2>
            <p style="color: #999; font-size: 0.9em; margin-bottom: 10px;">⚠️ After compression, paste base64 here to decompress</p>
            <textarea id="decompressInput" placeholder="Paste base64 compressed data..." rows="4"></textarea>
            <button onclick="decompress()">Decompress</button>
            <div id="decompressOutput" class="output"></div>
          </div>

          <h2>Test Cases</h2>
          <p style="margin: 15px 0; color: #666;">
            <a href="javascript:testCompression('Short text')">Short text</a> |
            <a href="javascript:testCompression('Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur.')">Long text</a> |
            <a href="javascript:testCompression(generateJSON())">JSON data</a>
          </p>
        </div>

        <script>
          function generateJSON() {
            const data = {
              items: Array.from({length: 50}, (_, i) => ({
                id: i,
                name: 'Item ' + i,
                value: Math.random() * 1000,
                timestamp: new Date().toISOString()
              }))
            };
            return JSON.stringify(data, null, 2);
          }

          async function compress() {
            const input = document.getElementById('compressInput').value;
            const format = localStorage.getItem('compressFormat') || 'gzip';

            try {
              const response = await fetch('/api/compress?format=' + format, {
                method: 'POST',
                body: input
              });
              const data = await response.json();
              document.getElementById('compressOutput').textContent = JSON.stringify(data, null, 2);
              document.getElementById('decompressInput').value = data.compressed_base64;
            } catch (e) {
              document.getElementById('compressOutput').textContent = 'Error: ' + e.message;
            }
          }

          async function decompress() {
            const input = document.getElementById('decompressInput').value;
            const format = localStorage.getItem('compressFormat') || 'gzip';

            try {
              const response = await fetch('/api/decompress?format=' + format, {
                method: 'POST',
                body: JSON.stringify({ compressed_base64: input })
              });
              const data = await response.json();
              document.getElementById('decompressOutput').textContent = JSON.stringify(data, null, 2);
            } catch (e) {
              document.getElementById('decompressOutput').textContent = 'Error: ' + e.message;
            }
          }

          function testCompression(text) {
            document.getElementById('compressInput').value = text;
            compress();
          }

          // Restore format selection
          const savedFormat = localStorage.getItem('compressFormat');
          if (savedFormat) {
            document.getElementById('compressFormat').value = savedFormat;
          }
        </script>
      </body>
      </html>
    `;
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }

  return new Response("Not found", { status: 404 });
});
