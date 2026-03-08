import { Buffer } from "node:buffer";

type Encoding = 'hex' | 'base64' | 'utf8' | 'utf-8' | 'latin1' | 'ascii';

function runtimeCore(): { ops?: Record<string, (...args: unknown[]) => unknown> } {
  return (globalThis as unknown as {
    Deno?: { core?: { ops?: Record<string, (...args: unknown[]) => unknown> } };
    __bootstrap?: { core?: { ops?: Record<string, (...args: unknown[]) => unknown> } };
  }).Deno?.core ??
    (globalThis as unknown as {
      __bootstrap?: { core?: { ops?: Record<string, (...args: unknown[]) => unknown> } };
    }).__bootstrap?.core ?? {};
}

function mapAlgorithm(name: string): string {
  const map: Record<string, string> = {
    'sha1': 'SHA-1', 'sha224': 'SHA-224', 'sha256': 'SHA-256',
    'sha384': 'SHA-384', 'sha512': 'SHA-512', 'md5': 'MD5',
  };
  return map[name.toLowerCase()] || name.toUpperCase();
}

function encodeData(data: string | Uint8Array | Buffer, encoding?: Encoding): Uint8Array {
  if (data instanceof Uint8Array) return data;
  if (Buffer.isBuffer(data)) return new Uint8Array(data);
  if (typeof data === 'string') return new Uint8Array(Buffer.from(data, (encoding || 'utf8') as any));
  return new Uint8Array(data);
}

function encodeOutput(data: Uint8Array, encoding?: Encoding): string | Buffer {
  if (!encoding) return Buffer.from(data);
  return Buffer.from(data).toString(encoding as any);
}

function resolveWebCrypto(): Crypto {
  const cryptoObj = (globalThis as { crypto?: Crypto }).crypto;
  if (!cryptoObj) {
    throw new Error('web crypto api is not available');
  }
  return cryptoObj;
}

// ============ Random Bytes (uses WebCrypto) ============

export function randomBytes(size: number): Buffer;
export function randomBytes(size: number, cb: (err: Error | null, buf: Buffer) => void): void;
export function randomBytes(size: number, cb?: (err: Error | null, buf: Buffer) => void): Buffer | void {
  if (size < 0 || size > 2147483647) {
    const err = new RangeError('invalid size');
    if (cb) { queueMicrotask(() => cb(err)); return; }
    throw err;
  }
  const buf = Buffer.from(resolveWebCrypto().getRandomValues(new Uint8Array(size)));
  if (cb) { queueMicrotask(() => cb(null, buf)); return; }
  return buf;
}

export function randomFillSync(buf: Uint8Array | Buffer, offset = 0, size?: number): Uint8Array | Buffer {
  const len = size ?? (buf.length - offset);
  if (offset < 0 || len < 0 || offset + len > buf.length) throw new RangeError('invalid offset/size');
  const target = new Uint8Array(buf.buffer, buf.byteOffset + offset, len);
  resolveWebCrypto().getRandomValues(target);
  return buf;
}

export function randomFill(buf: Uint8Array | Buffer, offset?: number | ((err: Error | null, buf: Buffer) => void),
  size?: number | ((err: Error | null, buf: Buffer) => void), cb?: (err: Error | null, buf: Buffer) => void): void {
  let actualOffset = 0, actualSize = buf.length, actualCallback: any;
  if (typeof offset === 'function') { actualCallback = offset; }
  else if (typeof size === 'function') { actualCallback = size; actualOffset = (offset as number) || 0; }
  else { actualCallback = cb; actualOffset = (offset as number) || 0; actualSize = (size as number) || (buf.length - actualOffset); }
  queueMicrotask(() => {
    try { randomFillSync(buf, actualOffset, actualSize); actualCallback?.(null, Buffer.from(buf)); }
    catch (err) { actualCallback?.(err as Error); }
  });
}

// ============ Hash (uses native Rust ops - SYNCHRONOUS) ============

export class Hash {
  #algo: string;
  #chunks: Uint8Array[] = [];

  constructor(algorithm: string) {
    this.#algo = mapAlgorithm(algorithm);
  }

  update(data: string | Uint8Array | Buffer, encoding?: Encoding): this {
    this.#chunks.push(encodeData(data, encoding));
    return this;
  }

  // [SYNC] NOW SYNCHRONOUS - calls native Rust op
  digest(encoding?: Encoding): string | Buffer {
    const op = runtimeCore().ops?.op_edge_crypto_hash;
    if (!op) {
      throw new Error('crypto native ops not available');
    }

    const combined = new Uint8Array(this.#chunks.reduce((a, c) => a + c.length, 0));
    let offset = 0;
    for (const chunk of this.#chunks) {
      combined.set(chunk, offset);
      offset += chunk.length;
    }

    // Call native Rust op - synchronous!
    const hashBytes = op(this.#algo, combined);
    return encodeOutput(new Uint8Array(hashBytes), encoding);
  }
}

// ============ HMAC (uses native Rust ops - SYNCHRONOUS) ============

export class Hmac {
  #algo: string;
  #key: Uint8Array;
  #chunks: Uint8Array[] = [];

  constructor(algorithm: string, key: string | Uint8Array | Buffer) {
    this.#algo = mapAlgorithm(algorithm);
    this.#key = encodeData(key);
  }

  update(data: string | Uint8Array | Buffer, encoding?: Encoding): this {
    this.#chunks.push(encodeData(data, encoding));
    return this;
  }

  // [SYNC] NOW SYNCHRONOUS - calls native Rust op
  digest(encoding?: Encoding): string | Buffer {
    const op = runtimeCore().ops?.op_edge_crypto_hmac;
    if (!op) {
      throw new Error('crypto native ops not available');
    }

    const combined = new Uint8Array(this.#chunks.reduce((a, c) => a + c.length, 0));
    let offset = 0;
    for (const chunk of this.#chunks) {
      combined.set(chunk, offset);
      offset += chunk.length;
    }

    // Call native Rust op - synchronous!
    const signature = op(this.#algo, this.#key, combined);
    return encodeOutput(new Uint8Array(signature), encoding);
  }
}

// ============ Factory Functions ============

export function createHash(algorithm: string): Hash {
  return new Hash(algorithm);
}

export function createHmac(algorithm: string, key: string | Uint8Array | Buffer): Hmac {
  return new Hmac(algorithm, key);
}

// ============ Unimplemented Functions ============

export function createCipher(): never {
  throw new Error('crypto.createCipher is deprecated');
}

export function createCipheriv(): never {
  throw new Error('[thunder] crypto.createCipheriv is not implemented in this runtime profile');
}

export function createDecipheriv(): never {
  throw new Error('[thunder] crypto.createDecipheriv is not implemented in this runtime profile');
}

export function pbkdf2(): never {
  throw new Error('crypto.pbkdf2 requires async - use WebCrypto or native binding');
}

export function pbkdf2Sync(): never {
  throw new Error('crypto.pbkdf2Sync requires native binding');
}

export function scrypt(): never {
  throw new Error('[thunder] crypto.scrypt is not implemented in this runtime profile');
}

export function scryptSync(): never {
  throw new Error('[thunder] crypto.scryptSync is not implemented in this runtime profile');
}

// ============ Exports ============

export const constants = { DEFAULT_ENCODING: 'utf8' };
export const webcrypto = (globalThis as { crypto?: Crypto }).crypto;

export default {
  randomBytes,
  randomFill,
  randomFillSync,
  createHash,
  createHmac,
  createCipher,
  createCipheriv,
  createDecipheriv,
  pbkdf2,
  pbkdf2Sync,
  scrypt,
  scryptSync,
  Hash,
  Hmac,
  constants,
  webcrypto,
};
