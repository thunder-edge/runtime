function open(): void {
  // No-op in this runtime profile.
}

function close(): void {
  // No-op in this runtime profile.
}

function url(): undefined {
  return undefined;
}

class Session {
  connect(): void {}
  disconnect(): void {}
  post(_method: string, _params?: unknown, cb?: (...args: unknown[]) => void): void {
    if (typeof cb === "function") {
      cb(new Error("[thunder] inspector.Session.post is not implemented in this runtime profile"));
    }
  }
}

const inspectorModule = { open, close, url, Session };

export { open, close, url, Session };
export default inspectorModule;
