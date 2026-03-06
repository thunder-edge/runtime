# Streaming de Response Body

Este runtime suporta resposta HTTP por stream sem bufferizar todo o corpo em memoria no roteador HTTP.

## O que mudou

- `ReadableStream` retornado pelo handler agora pode ser enviado como resposta chunked.
- Fluxos de SSE (Server-Sent Events) passam a funcionar sem precisar montar uma string gigante.
- Respostas tradicionais (string/JSON) continuam funcionando normalmente.

## Exemplo 1: Chunked streaming (TS)

```ts
Deno.serve((_req) => {
  const encoder = new TextEncoder();

  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(encoder.encode("chunk-1\n"));
      controller.enqueue(encoder.encode("chunk-2\n"));
      controller.enqueue(encoder.encode("chunk-3\n"));
      controller.close();
    },
  });

  return new Response(body, {
    status: 200,
    headers: {
      "content-type": "text/plain; charset=utf-8",
      "transfer-encoding": "chunked",
    },
  });
});
```

## Exemplo 2: SSE com eventos periodicos (TS/JS)

```ts
Deno.serve((_req) => {
  const encoder = new TextEncoder();

  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      let n = 0;

      const timer = setInterval(() => {
        n += 1;
        controller.enqueue(encoder.encode(`event: tick\ndata: ${n}\n\n`));

        if (n >= 5) {
          clearInterval(timer);
          controller.close();
        }
      }, 1000);
    },
    cancel() {
      // opcional: aqui voce limparia recursos extras se necessario
    },
  });

  return new Response(body, {
    headers: {
      "content-type": "text/event-stream",
      "cache-control": "no-cache",
      connection: "keep-alive",
    },
  });
});
```

## Exemplo 3: Streaming de dados gerados em loop

```ts
Deno.serve(async (_req) => {
  const encoder = new TextEncoder();

  async function* generateLines() {
    for (let i = 0; i < 10; i++) {
      yield encoder.encode(`line ${i}\n`);
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }

  const iterator = generateLines();
  const body = new ReadableStream<Uint8Array>({
    async pull(controller) {
      const next = await iterator.next();
      if (next.done) {
        controller.close();
      } else {
        controller.enqueue(next.value);
      }
    },
  });

  return new Response(body, {
    headers: { "content-type": "text/plain; charset=utf-8" },
  });
});
```

## Como validar localmente

1. Bundle da funcao.
2. Deploy via endpoint interno `/_internal/functions`.
3. Chame o endpoint da funcao com `curl -N` para nao bufferizar no cliente.

Exemplo:

```bash
curl -N http://127.0.0.1:9001/minha-funcao/stream
```

Para SSE:

```bash
curl -N -H 'Accept: text/event-stream' http://127.0.0.1:9001/minha-funcao/events
```

## Observacoes

- O timeout da funcao continua sendo aplicado. Streams longos devem ajustar `wall_clock_timeout_ms` no deploy.
- Para SSE, prefira enviar mensagens pequenas e frequentes.
- Caso o cliente desconecte, o stream encerra no lado HTTP automaticamente.
