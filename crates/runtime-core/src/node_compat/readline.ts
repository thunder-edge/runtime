import { EventEmitter } from "node:events";

type NodeLikeError = Error & { code?: string };

function notImplemented(api: string): never {
  const err = new Error(
    `[thunder] ${api} is not implemented in this runtime profile`,
  ) as NodeLikeError;
  err.code = "ERR_NOT_IMPLEMENTED";
  throw err;
}

class Interface extends EventEmitter {
  question(_query: string, _cb?: (answer: string) => void): never {
    return notImplemented("readline.Interface.question");
  }

  close(): void {
    this.emit("close");
  }
}

function createInterface(): Interface {
  return new Interface();
}

function clearLine(): never {
  return notImplemented("readline.clearLine");
}

const readlineModule = { Interface, createInterface, clearLine };

export { Interface, createInterface, clearLine };
export default readlineModule;
