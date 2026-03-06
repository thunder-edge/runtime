Deno.serve((req) => {
  return new Response("Hello from edge function!", {
    headers: { "content-type": "text/plain" },
  });
});
