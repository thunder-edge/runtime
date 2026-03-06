// Example: WebSocket
// Real-time communication with WebSocket

Deno.serve((req) => {
  // Upgrade the connection to WebSocket
  const { socket, response } = Deno.upgradeWebSocket(req);

  socket.onmessage = (event) => {
    try {
      const message = event.data;

      if (typeof message === "string") {
        socket.send(
          JSON.stringify({
            type: "echo",
            message,
            receivedAt: new Date().toISOString(),
          })
        );

        if (message.toLowerCase().includes("hello")) {
          socket.send(
            JSON.stringify({
              type: "greeting",
              message: "Hello there! How can I help?",
            })
          );
        }
      } else if (message instanceof Blob) {
        message.arrayBuffer().then((buffer) => {
          socket.send(
            JSON.stringify({
              type: "binary",
              size: buffer.byteLength,
            })
          );
        });
      } else if (message instanceof ArrayBuffer) {
        socket.send(
          JSON.stringify({
            type: "binary",
            size: message.byteLength,
          })
        );
      }
    } catch (error) {
      console.error("WebSocket message error:", error);
    }
  };

  socket.onerror = (error) => {
    console.error("WebSocket error:", error);
  };

  return response;
});
