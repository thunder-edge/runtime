// Example: AbortController - Request Cancellation
// Demonstrates cancelling requests using AbortSignal

Deno.serve(async (req) => {
  const url = new URL(req.url);

  // Slow endpoint that can be cancelled
  if (url.pathname === "/api/slow" && req.method === "GET") {
    const duration = parseInt(url.searchParams.get("duration") || "10");

    const stream = new ReadableStream({
      async start(controller) {
        try {
          for (let i = 0; i < duration; i++) {
            if (req.signal.aborted) {
              controller.error(new Error("Request was cancelled"));
              return;
            }

            const message = JSON.stringify({
              progress: ((i + 1) / duration * 100).toFixed(0),
              elapsed: i + 1,
              total: duration,
              timestamp: new Date().toISOString(),
            }) + "\n";

            controller.enqueue(message);
            await new Promise((resolve) => setTimeout(resolve, 1000));
          }

          controller.enqueue(
            JSON.stringify({
              status: "completed",
              message: "Operation completed successfully",
              timestamp: new Date().toISOString(),
            }) + "\n"
          );

          controller.close();
        } catch (error) {
          controller.error(error);
        }
      },
    });

    return new Response(stream, {
      headers: {
        "content-type": "application/x-ndjson",
        "transfer-encoding": "chunked",
      },
    });
  }

  // Fetch with timeout and cancellation
  if (url.pathname === "/api/fetch-with-timeout" && req.method === "POST") {
    try {
      const { url: fetchUrl, timeout = 5000 } = await req.json();

      const controller = new AbortController();
      const timeoutId = setTimeout(() => controller.abort(), timeout);

      try {
        const response = await fetch(fetchUrl, {
          signal: controller.signal,
        });

        clearTimeout(timeoutId);

        const body = await response.text();
        return new Response(
          JSON.stringify({
            success: true,
            status: response.status,
            headers: Object.fromEntries(response.headers),
            bodyLength: body.length,
            timeout: false,
          }),
          {
            headers: { "content-type": "application/json" },
          }
        );
      } catch (error) {
        clearTimeout(timeoutId);

        if ((error as Error)?.name === "AbortError") {
          return new Response(
            JSON.stringify({
              error: "Request timeout",
              timeout: true,
              timeoutMs: timeout,
              url: fetchUrl,
            }),
            {
              status: 408,
              headers: { "content-type": "application/json" },
            }
          );
        }
        throw error;
      }
    } catch (error) {
      return new Response(
        JSON.stringify({
          error: "Fetch failed",
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
        <title>AbortController</title>
        <style>
          * { margin: 0; padding: 0; box-sizing: border-box; }
          body { font-family: Arial; background: #f5f5f5; padding: 40px 20px; }
          .container { max-width: 900px; margin: 0 auto; background: white; border-radius: 8px; padding: 40px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
          h1 { color: #333; margin-bottom: 10px; }
          .subtitle { color: #999; margin-bottom: 30px; }
          .section { background: #f9f9f9; border-left: 4px solid #667eea; padding: 20px; margin: 20px 0; border-radius: 4px; }
          h2 { color: #667eea; margin: 20px 0 15px; }
          input, select { width: 100%; padding: 10px; margin: 5px 0; border: 1px solid #ddd; border-radius: 4px; }
          button { background: #667eea; color: white; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; margin: 10px 5px 10px 0; }
          button:hover { background: #764ba2; }
          button:disabled { background: #ccc; cursor: not-allowed; }
          .progress-container { background: white; border-radius: 4px; padding: 15px; margin: 15px 0; border: 1px solid #ddd; }
          .progress-bar { width: 100%; height: 25px; background: #e0e0e0; border-radius: 12px; overflow: hidden; }
          .progress-fill { height: 100%; background: linear-gradient(90deg, #667eea, #764ba2); transition: width 0.3s; display: flex; align-items: center; justify-content: center; color: white; font-size: 0.8em; font-weight: bold; }
          .output { background: white; padding: 15px; border: 1px solid #ddd; border-radius: 4px; margin: 15px 0; max-height: 300px; overflow-y: auto; font-family: monospace; font-size: 0.9em; }
          .message { padding: 10px; margin: 5px 0; border-radius: 4px; border-left: 4px solid #667eea; }
          .message.success { background: #d4f1d4; border-left-color: #4CAF50; }
          .message.error { background: #f8d7da; border-left-color: #f44336; }
          .message.info { background: #d1ecf1; border-left-color: #4a90e2; }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>🛑 AbortController - Request Cancellation</h1>
          <p class="subtitle">Demonstrate cancelling requests using AbortSignal</p>

          <div class="section">
            <h2>1. Streaming Request (Cancellable)</h2>
            <p style="margin: 10px 0; color: #666;">
              This endpoint streams progress for N seconds. You can cancel it at any time.
            </p>
            <label style="display: block; margin: 10px 0;">Duration (seconds):</label>
            <input type="number" id="duration" value="10" min="1" max="60">
            <button id="startBtn" onclick="startStream()">Start Stream</button>
            <button id="cancelBtn" onclick="cancelStream()" disabled style="background: #f44336;">Cancel</button>

            <div class="progress-container">
              <div class="progress-bar">
                <div class="progress-fill" id="progressBar" style="width: 0%;">0%</div>
              </div>
            </div>

            <div id="streamOutput" class="output"></div>
          </div>

          <div class="section">
            <h2>2. Fetch with Request Timeout</h2>
            <p style="margin: 10px 0; color: #666;">
              Test request cancellation when fetch takes too long.
            </p>
            <input type="url" id="fetchUrl" placeholder="Enter URL to fetch..." value="https://httpbin.org/delay/10">
            <label style="display: block; margin: 10px 0;">Timeout (ms):</label>
            <input type="number" id="timeout" value="5000" min="1000" step="1000">
            <button onclick="testFetchTimeout()">Test Fetch</button>

            <div id="fetchOutput" class="output"></div>
          </div>

          <div class="section">
            <h2>How AbortController Works</h2>
            <ul style="margin-left: 20px; line-height: 1.8; color: #666;">
              <li><strong>AbortController:</strong> Creates a controller to abort associated signals</li>
              <li><strong>signal:</strong> Passed to fetch, fetch, ReadableStream, etc.</li>
              <li><strong>abort():</strong> Cancels the operation</li>
              <li><strong>aborted:</strong> Check if cancellation was requested</li>
            </ul>
          </div>
        </div>

        <script>
          let currentAbortController = null;

          async function startStream() {
            currentAbortController = new AbortController();
            const duration = document.getElementById('duration').value;
            const output = document.getElementById('streamOutput');
            output.innerHTML = '';

            document.getElementById('startBtn').disabled = true;
            document.getElementById('cancelBtn').disabled = false;

            try {
              const response = await fetch(\`/api/slow?duration=\${duration}\`, {
                signal: currentAbortController.signal
              });

              const reader = response.body.getReader();
              const decoder = new TextDecoder();

              while (true) {
                const { done, value } = await reader.read();
                if (done) break;

                const text = decoder.decode(value, { stream: true });
                const lines = text.trim().split('\\n');

                for (const line of lines) {
                  if (!line) continue;
                  const data = JSON.parse(line);
                  updateProgress(data);
                  addMessage('info', \`Progress: \${data.progress}%\`);
                }
              }

              addMessage('success', 'Operation completed successfully');
            } catch (error) {
              if (error.name === 'AbortError') {
                addMessage('error', 'Request was cancelled');
              } else {
                addMessage('error', \`Error: \${error.message}\`);
              }
            } finally {
              document.getElementById('startBtn').disabled = false;
              document.getElementById('cancelBtn').disabled = true;
            }
          }

          function cancelStream() {
            if (currentAbortController) {
              currentAbortController.abort();
              addMessage('error', 'Cancellation requested');
            }
          }

          function updateProgress(data) {
            const progress = parseInt(data.progress) || 0;
            const bar = document.getElementById('progressBar');
            bar.style.width = progress + '%';
            bar.textContent = progress + '%';
          }

          async function testFetchTimeout() {
            const fetchUrl = document.getElementById('fetchUrl').value;
            const timeout = parseInt(document.getElementById('timeout').value);
            const output = document.getElementById('fetchOutput');
            output.innerHTML = '';

            addMessage('info', \`Fetching \${fetchUrl} with \${timeout}ms timeout...\`);

            try {
              const response = await fetch('/api/fetch-with-timeout', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ url: fetchUrl, timeout })
              });

              const data = await response.json();

              if (response.ok) {
                addMessage('success', JSON.stringify(data, null, 2));
              } else {
                addMessage('error', JSON.stringify(data, null, 2));
              }
            } catch (error) {
              addMessage('error', \`Error: \${error.message}\`);
            }
          }

          function addMessage(type, message) {
            const output = document.getElementById('streamOutput') || document.getElementById('fetchOutput');
            const msgEl = document.createElement('div');
            msgEl.className = 'message ' + type;
            msgEl.textContent = message;
            output.appendChild(msgEl);
            output.scrollTop = output.scrollHeight;
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
