// Example: WebSocket
// Real-time communication with WebSocket

Deno.serve((req) => {
  // Upgrade the connection to WebSocket
  const { socket, response } = Deno.upgrade(req);

  (async () => {
    try {
      for await (const message of socket) {
        if (typeof message === "string") {
          // Echo the message back
          const response = {
            type: "echo",
            message: message,
            receivedAt: new Date().toISOString(),
          };
          socket.send(JSON.stringify(response));

          // Send additional info
          if (message.toLowerCase().includes("hello")) {
            socket.send(
              JSON.stringify({
                type: "greeting",
                message: "Hello there! How can I help?",
              })
            );
          }
        } else if (message instanceof Uint8Array) {
          // Handle binary data
          const text = new TextDecoder().decode(message);
          socket.send(
            JSON.stringify({
              type: "binary",
              message: text,
              size: message.byteLength,
            })
          );
        }
      }
    } catch (error) {
      console.error("WebSocket error:", error);
    }
  })();

  return response;
});
