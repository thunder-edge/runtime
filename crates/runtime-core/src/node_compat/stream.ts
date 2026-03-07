import { EventEmitter } from "node:events";

type Callback = (err?: unknown, value?: unknown) => void;

type AbortSignalLike = {
  aborted: boolean;
  reason?: unknown;
  addEventListener: (type: string, listener: () => void, options?: unknown) => void;
  removeEventListener: (type: string, listener: () => void) => void;
};

type PipelineOptions = {
  signal?: AbortSignalLike;
};

class Stream extends EventEmitter {}

class Readable extends Stream {
  readable = true;
  destroyed = false;
  readableEnded = false;
  #paused = false;
  #buffer: unknown[] = [];
  #highWaterMark: number;

  constructor(options: Record<string, unknown> = {}) {
    super();
    this.#highWaterMark = (options.highWaterMark as number) || 16384;
  }

  static from(iterable: Iterable<unknown> | AsyncIterable<unknown>) {
    const readable = new Readable();

    const pump = async () => {
      try {
        for await (const chunk of iterable as AsyncIterable<unknown>) {
          readable.push(chunk);
        }
        readable.push(null);
      } catch (err) {
        readable.emit("error", err);
      }
    };

    queueMicrotask(() => {
      void pump();
    });

    return readable;
  }

  push(chunk: unknown) {
    if (this.destroyed) return false;

    if (chunk === null) {
      this.readableEnded = true;
      this.emit("end");
      this.emit("close");
      return false;
    }

    // If paused or buffer is full, queue the chunk
    if (this.#paused || this.#buffer.length >= this.#highWaterMark) {
      this.#buffer.push(chunk);
      return false; // Signal backpressure
    }

    // Emit data immediately if not paused and buffer is not full
    this.emit("data", chunk);

    // If buffer has accumulated data, it means we hit highWaterMark on previous call
    // Return false to signal backpressure
    return this.#buffer.length === 0;
  }

  pause() {
    this.#paused = true;
    this.emit("pause");
    return this;
  }

  resume() {
    this.#paused = false;
    this.emit("resume");

    // Flush internal buffer while not paused
    while (!this.#paused && this.#buffer.length > 0) {
      const chunk = this.#buffer.shift();
      if (chunk === null) {
        this.readableEnded = true;
        this.emit("end");
        this.emit("close");
        break;
      } else {
        this.emit("data", chunk);
      }
    }

    return this;
  }

  pipe(destination: Writable | Transform | Duplex) {
    const onData = (chunk: unknown) => {
      const canContinue = destination.write(chunk);
      if (!canContinue) {
        this.pause();
      }
    };

    const onDrain = () => {
      this.resume();
    };

    this.on("data", onData);
    this.on("end", () => {
      destination.end();
    });
    this.on("error", (err: unknown) => {
      destination.emit("error", err);
    });

    destination.on("drain", onDrain);

    return destination;
  }

  destroy(error?: unknown) {
    this.destroyed = true;
    if (error !== undefined) {
      this.emit("error", error);
    }
    this.emit("close");
    return this;
  }
}

class Writable extends Stream {
  writable = true;
  destroyed = false;
  writableEnded = false;
  #writeImpl?: (chunk: unknown, encoding: string, cb: Callback) => void;
  #buffer: Array<{ data: unknown; encoding: string; size: number }> = [];
  #highWaterMark: number;
  #writing = false;
  #bufferedBytes = 0;
  #ending = false;
  #endCallbacks: Array<() => void> = [];

  constructor(options: Record<string, unknown> = {}) {
    super();
    this.#writeImpl = options.write as
      | ((chunk: unknown, encoding: string, cb: Callback) => void)
      | undefined;
    this.#highWaterMark = (options.highWaterMark as number) || 16384;
  }

  write(chunk: unknown, encodingOrCb?: string | Callback, maybeCb?: Callback) {
    if (this.destroyed || this.writableEnded || this.#ending) return false;

    const encoding = typeof encodingOrCb === "string" ? encodingOrCb : "utf8";
    const cb = (typeof encodingOrCb === "function" ? encodingOrCb : maybeCb) ?? (() => {});
    const size = byteLengthOfChunk(chunk, encoding);

    const done = () => {
      cb();
      this.#writing = false;

      // Flush buffer if there's more data
      if (this.#buffer.length > 0) {
        const nextChunk = this.#buffer.shift()!;
        this.#bufferedBytes = Math.max(0, this.#bufferedBytes - nextChunk.size);
        this.write(nextChunk.data, nextChunk.encoding);
        return;
      }

      // Emit drain when buffer is flushed
      if (this.#buffer.length === 0) {
        queueMicrotask(() => this.emit("drain"));
      }

      if (this.#ending) {
        this.#finalizeEnd();
      }
    };

    // If already writing, buffer the chunk
    if (this.#writing || this.#buffer.length > 0) {
      this.#buffer.push({ data: chunk, encoding, size });
      this.#bufferedBytes += size;
      return this.#bufferedBytes < this.#highWaterMark;
    }

    this.#writing = true;

    if (this.#writeImpl) {
      this.#writeImpl(chunk, encoding, done);
    } else {
      done();
    }

    return this.#bufferedBytes < this.#highWaterMark;
  }

  end(chunkOrCb?: unknown, encodingOrCb?: string | Callback, maybeCb?: Callback) {
    if (typeof chunkOrCb === "function") {
      chunkOrCb();
    } else if (chunkOrCb !== undefined) {
      this.write(chunkOrCb, encodingOrCb as string | Callback, maybeCb);
    }

    const cb =
      (typeof encodingOrCb === "function" ? encodingOrCb : maybeCb) ??
      (typeof chunkOrCb === "function" ? chunkOrCb : undefined);

    if (typeof cb === "function") {
      this.#endCallbacks.push(() => cb());
    }

    this.#ending = true;
    if (!this.#writing && this.#buffer.length === 0) {
      this.#finalizeEnd();
    }
    return this;
  }

  #finalizeEnd() {
    if (this.writableEnded) return;
    this.writableEnded = true;
    this.emit("finish");
    this.emit("close");
    for (const cb of this.#endCallbacks) {
      cb();
    }
    this.#endCallbacks = [];
  }

  destroy(error?: unknown) {
    this.destroyed = true;
    if (error !== undefined) {
      this.emit("error", error);
    }
    this.emit("close");
    return this;
  }
}

class Duplex extends Readable {
  writable = true;
  writableEnded = false;
  #writeImpl?: (chunk: unknown, encoding: string, cb: Callback) => void;

  constructor(options: Record<string, unknown> = {}) {
    super(options);
    this.#writeImpl = options.write as
      | ((chunk: unknown, encoding: string, cb: Callback) => void)
      | undefined;
  }

  write(chunk: unknown, encodingOrCb?: string | Callback, maybeCb?: Callback) {
    if (this.writableEnded) return false;

    const encoding = typeof encodingOrCb === "string" ? encodingOrCb : "utf8";
    const cb = (typeof encodingOrCb === "function" ? encodingOrCb : maybeCb) ?? (() => {});

    if (this.#writeImpl) {
      this.#writeImpl(chunk, encoding, cb);
    } else {
      cb();
    }

    return true;
  }

  end(chunkOrCb?: unknown, encodingOrCb?: string | Callback, maybeCb?: Callback) {
    if (typeof chunkOrCb === "function") {
      chunkOrCb();
    } else if (chunkOrCb !== undefined) {
      this.write(chunkOrCb, encodingOrCb as string | Callback, maybeCb);
    }

    const cb =
      (typeof encodingOrCb === "function" ? encodingOrCb : maybeCb) ??
      (typeof chunkOrCb === "function" ? chunkOrCb : undefined);

    this.writableEnded = true;
    this.push(null);
    this.emit("finish");
    if (typeof cb === "function") cb();
    return this;
  }
}

class Transform extends Duplex {
  #transformImpl?: (
    chunk: unknown,
    encoding: string,
    cb: (err?: unknown, data?: unknown) => void,
  ) => void;

  constructor(options: Record<string, unknown> = {}) {
    super(options);
    this.#transformImpl = options.transform as
      | ((chunk: unknown, encoding: string, cb: (err?: unknown, data?: unknown) => void) => void)
      | undefined;
  }

  write(chunk: unknown, encodingOrCb?: string | Callback, maybeCb?: Callback) {
    if (this.writableEnded) return false;

    const encoding = typeof encodingOrCb === "string" ? encodingOrCb : "utf8";
    const cb = (typeof encodingOrCb === "function" ? encodingOrCb : maybeCb) ?? (() => {});

    const done = (err?: unknown, data?: unknown) => {
      if (err !== undefined) {
        this.emit("error", err);
        cb(err);
        return;
      }
      if (data !== undefined && data !== null) {
        this.push(data);
      }
      cb();
    };

    if (this.#transformImpl) {
      this.#transformImpl(chunk, encoding, done);
    } else {
      done(undefined, chunk);
    }

    return true;
  }
}

class PassThrough extends Transform {
  constructor(options: Record<string, unknown> = {}) {
    super({
      ...options,
      transform: (chunk: unknown, _encoding: string, cb: (err?: unknown, data?: unknown) => void) => {
        cb(undefined, chunk);
      },
    });
  }
}

function byteLengthOfChunk(chunk: unknown, encoding: string): number {
  if (chunk === null || chunk === undefined) return 0;
  if (typeof chunk === "string") {
    return Buffer.byteLength(chunk, encoding as BufferEncoding);
  }
  if (chunk instanceof Uint8Array) {
    return chunk.byteLength;
  }
  if (ArrayBuffer.isView(chunk)) {
    return chunk.byteLength;
  }
  if (chunk instanceof ArrayBuffer) {
    return chunk.byteLength;
  }
  return Buffer.byteLength(String(chunk), encoding as BufferEncoding);
}

function pipeline(...streamsOrCb: unknown[]) {
  let options: PipelineOptions | undefined;
  const cb = typeof streamsOrCb[streamsOrCb.length - 1] === "function"
    ? (streamsOrCb.pop() as (err?: unknown) => void)
    : undefined;

  const maybeOptions = streamsOrCb[streamsOrCb.length - 1];
  if (
    maybeOptions &&
    typeof maybeOptions === "object" &&
    "signal" in (maybeOptions as Record<string, unknown>)
  ) {
    options = streamsOrCb.pop() as PipelineOptions;
  }

  const streams = streamsOrCb as Array<Readable | Writable | Transform | Duplex>;

  if (streams.length < 2) {
    if (cb) cb(new Error("pipeline requires at least two streams"));
    return undefined;
  }

  for (let i = 0; i < streams.length - 1; i++) {
    streams[i].pipe(streams[i + 1] as Writable | Transform | Duplex);
  }

  const last = streams[streams.length - 1] as Writable;

  let settled = false;
  const done = (err?: unknown) => {
    if (settled) return;
    settled = true;

    if (signal && onAbort) {
      signal.removeEventListener("abort", onAbort);
    }

    if (cb) cb(err);
  };

  const handleStreamError = (err: unknown) => {
    done(err);
  };

  for (const stream of streams) {
    stream.once("error", handleStreamError);
  }

  const signal = options?.signal;
  const toAbortError = () => {
    const reason = signal?.reason;
    if (reason instanceof Error) {
      return reason;
    }
    return new Error("The operation was aborted");
  };

  const abortAllStreams = (err: Error) => {
    for (const stream of streams) {
      if (typeof (stream as { destroy?: (error?: unknown) => void }).destroy === "function") {
        (stream as { destroy: (error?: unknown) => void }).destroy(err);
      }
    }
  };

  const onAbort = signal
    ? () => {
        const abortErr = toAbortError();
        abortAllStreams(abortErr);
        done(abortErr);
      }
    : undefined;

  if (signal?.aborted) {
    const abortErr = toAbortError();
    abortAllStreams(abortErr);
    done(abortErr);
    return last;
  }

  if (signal && onAbort) {
    signal.addEventListener("abort", onAbort, { once: true });
  }

  if (cb) {
    last.once("finish", () => done());
  }

  return last;
}

function finished(
  stream: Readable | Writable | Duplex | Transform,
  cb: (err?: unknown) => void,
) {
  let done = false;
  const onceDone = (err?: unknown) => {
    if (done) return;
    done = true;
    cb(err);
  };

  stream.once("end", () => onceDone());
  stream.once("finish", () => onceDone());
  stream.once("close", () => onceDone());
  stream.once("error", (err: unknown) => onceDone(err));
}

const promises = {
  pipeline: (...streams: unknown[]) =>
    new Promise<void>((resolve, reject) => {
      pipeline(...streams, (err?: unknown) => {
        if (err) reject(err);
        else resolve();
      });
    }),
};

const streamModule = {
  Stream,
  Readable,
  Writable,
  Duplex,
  Transform,
  PassThrough,
  pipeline,
  finished,
  promises,
};

export {
  Stream,
  Readable,
  Writable,
  Duplex,
  Transform,
  PassThrough,
  pipeline,
  finished,
  promises,
};

export default streamModule;
