type NodeLikeError = Error & { code?: string };

function notImplemented(api: string): never {
  const err = new Error(
    `[thunder] ${api} is not implemented in this runtime profile`,
  ) as NodeLikeError;
  err.code = "ERR_NOT_IMPLEMENTED";
  throw err;
}

class Script {
  code: string;

  constructor(code: string) {
    this.code = String(code);
  }

  runInThisContext(): never {
    return notImplemented("vm.Script.runInThisContext");
  }
}

function runInNewContext(): never {
  return notImplemented("vm.runInNewContext");
}

function createContext<T extends object>(context: T): T {
  return context;
}

const vmModule = { Script, runInNewContext, createContext };

export { Script, runInNewContext, createContext };
export default vmModule;
