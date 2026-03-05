// Example: Server-Sent Events (SSE)
// Real-time server-to-client communication

Deno.serve((req) => {
  const url = new URL(req.url);

  // SSE endpoint
  if (url.pathname === "/api/events" && req.method === "GET") {
    // Create a ReadableStream that sends events
    const stream = new ReadableStream({
      async start(controller) {
        // Helper to send SSE formatted message
        const sendEvent = (
          data: Record<string, unknown>,
          eventType: string = "message"
        ) => {
          const message = `event: ${eventType}\ndata: ${JSON.stringify(data)}\n\n`;
          controller.enqueue(message);
        };

        try {
          // Send initial connection event
          sendEvent(
            {
              timestamp: new Date().toISOString(),
              status: "connected",
              message: "Server-Sent Events connection established",
            },
            "connect"
          );

          // Send events at intervals
          for (let i = 0; i < 10; i++) {
            await new Promise((resolve) => setTimeout(resolve, 1000));

            sendEvent(
              {
                counter: i + 1,
                timestamp: new Date().toISOString(),
                data: `Event #${i + 1}`,
              },
              "update"
            );
          }

          // Send completion event
          sendEvent(
            {
              status: "completed",
              totalEvents: 10,
              timestamp: new Date().toISOString(),
            },
            "done"
          );

          controller.close();
        } catch (error) {
          sendEvent(
            {
              error: (error as Error)?.message,
              timestamp: new Date().toISOString(),
            },
            "error"
          );
          controller.close();
        }
      },
    });

    return new Response(stream, {
      headers: {
        "content-type": "text/event-stream",
        "cache-control": "no-cache",
        "connection": "keep-alive",
      },
    });
  }

  // Home page
  if (url.pathname === "/") {
    const html = `
      <!DOCTYPE html>
      <html>
      <head>
        <meta charset="UTF-8">
        <title>Server-Sent Events</title>
        <style>
          * { margin: 0; padding: 0; box-sizing: border-box; }
          body { font-family: Arial; background: #f5f5f5; padding: 40px 20px; }
          .container { max-width: 900px; margin: 0 auto; background: white; border-radius: 8px; padding: 40px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
          h1 { color: #333; margin-bottom: 10px; }
          .subtitle { color: #999; margin-bottom: 30px; }
          .section { background: #f9f9f9; border-left: 4px solid #667eea; padding: 20px; margin: 20px 0; border-radius: 4px; }
          button { background: #667eea; color: white; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; margin: 10px 0; }
          button:hover { background: #764ba2; }
          button:disabled { background: #ccc; cursor: not-allowed; }
          .event-log { background: white; border: 1px solid #ddd; border-radius: 4px; padding: 15px; margin-top: 15px; max-height: 400px; overflow-y: auto; font-family: monospace; font-size: 0.9em; }
          .event { padding: 10px; margin: 5px 0; border-radius: 4px; border-left: 4px solid #667eea; }
          .event.connect { background: #d4f1d4; border-left-color: #4CAF50; }
          .event.update { background: #d1ecf1; border-left-color: #4a90e2; }
          .event.done { background: #d4f1d4; border-left-color: #4CAF50; }
          .event.error { background: #f8d7da; border-left-color: #f44336; }
          .timestamp { color: #999; font-size: 0.85em; }
          .status { padding: 10px; border-radius: 4px; margin: 10px 0; }
          .status.connected { background: #d4f1d4; color: #2e7d32; }
          .status.disconnected { background: #f8d7da; color: #c41c3b; }
          .stats { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; margin: 15px 0; }
          .stat-box { background: white; padding: 15px; border-radius: 4px; border: 1px solid #ddd; text-align: center; }
          .stat-value { font-size: 2em; font-weight: bold; color: #667eea; }
          .stat-label { color: #999; font-size: 0.9em; margin-top: 5px; }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>📡 Server-Sent Events (SSE)</h1>
          <p class="subtitle">Real-time server-to-client communication using ReadableStream</p>

          <div class="section">
            <h2>Connection Controls</h2>
            <button id="connectBtn" onclick="connectSSE()">Connect</button>
            <button id="disconnectBtn" onclick="disconnectSSE()" disabled>Disconnect</button>

            <div id="status" class="status disconnected">
              Status: <strong id="statusText">Disconnected</strong>
            </div>

            <div class="stats">
              <div class="stat-box">
                <div class="stat-value" id="eventCount">0</div>
                <div class="stat-label">Events Received</div>
              </div>
              <div class="stat-box">
                <div class="stat-value" id="byteCount">0</div>
                <div class="stat-label">Bytes Received</div>
              </div>
            </div>
          </div>

          <div class="section">
            <h2>Event Log</h2>
            <button onclick="clearLog()">Clear Log</button>
            <div id="eventLog" class="event-log"></div>
          </div>

          <div class="section">
            <h2>How It Works</h2>
            <p style="line-height: 1.8; color: #666;">
              Server-Sent Events (SSE) provide an efficient way for servers to push real-time data to clients over a single HTTP connection. Unlike WebSockets, SSE is one-directional (server to client) and works with standard HTTP.
            </p>
            <ul style="margin-left: 20px; line-height: 1.8; color: #666;">
              <li><strong>event:</strong> Type of event (e.g., "message", "update", "error")</li>
              <li><strong>data:</strong> Event payload (typically JSON)</li>
              <li><strong>id:</strong> Optional unique event identifier</li>
              <li><strong>retry:</strong> Milliseconds before reconnect attempt</li>
            </ul>
          </div>
        </div>

        <script>
          let eventSource;
          let eventCount = 0;
          let byteCount = 0;

          async function connectSSE() {
            eventCount = 0;
            byteCount = 0;
            updateStats();
            clearLog();

            document.getElementById('connectBtn').disabled = true;
            document.getElementById('disconnectBtn').disabled = false;

            try {
              const response = await fetch('/api/events');
              const reader = response.body.getReader();
              const decoder = new TextDecoder();

              const status = document.getElementById('status');
              status.className = 'status connected';
              document.getElementById('statusText').textContent = 'Connected';

              (async () => {
                while (true) {
                  const { done, value } = await reader.read();
                  if (done) break;

                  const text = decoder.decode(value, { stream: true });
                  byteCount += value.byteLength;
                  processSSEData(text);
                  updateStats();
                }

                // Connection closed
                const status = document.getElementById('status');
                status.className = 'status disconnected';
                document.getElementById('statusText').textContent = 'Disconnected';
                document.getElementById('connectBtn').disabled = false;
                document.getElementById('disconnectBtn').disabled = true;
              })();
            } catch (error) {
              addLog('error', { error: error.message });
              document.getElementById('connectBtn').disabled = false;
              document.getElementById('disconnectBtn').disabled = true;
            }
          }

          function disconnectSSE() {
            if (eventSource) {
              eventSource.close();
            }
            document.getElementById('connectBtn').disabled = false;
            document.getElementById('disconnectBtn').disabled = true;

            const status = document.getElementById('status');
            status.className = 'status disconnected';
            document.getElementById('statusText').textContent = 'Disconnected';
          }

          function processSSEData(data) {
            const lines = data.trim().split('\\n');
            let eventType = 'message';
            let eventData = '';

            for (const line of lines) {
              if (line.startsWith('event: ')) {
                eventType = line.substring(7);
              } else if (line.startsWith('data: ')) {
                eventData = line.substring(6);
              }
            }

            if (eventData) {
              try {
                const parsed = JSON.parse(eventData);
                addLog(eventType, parsed);
                eventCount++;
              } catch (e) {
                addLog(eventType, { raw: eventData });
              }
            }
          }

          function addLog(type, data) {
            const log = document.getElementById('eventLog');
            const eventEl = document.createElement('div');
            eventEl.className = 'event ' + type;
            eventEl.innerHTML = \`
              <div>
                <strong>[\${type.toUpperCase()}]</strong>
                <span class="timestamp">\${new Date().toLocaleTimeString()}</span>
              </div>
              <div style="margin-top: 5px; font-size: 0.95em;">
                \${JSON.stringify(data, null, 2)}
              </div>
            \`;
            log.appendChild(eventEl);
            log.scrollTop = log.scrollHeight;
          }

          function clearLog() {
            document.getElementById('eventLog').innerHTML = '';
          }

          function updateStats() {
            document.getElementById('eventCount').textContent = eventCount;
            document.getElementById('byteCount').textContent = byteCount.toLocaleString();
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
