type NodeLikeError = Error & { code?: string };

function notImplemented(api: string): never {
  const err = new Error(
    `[thunder] ${api} is not implemented in this runtime profile`,
  ) as NodeLikeError;
  err.code = "ERR_NOT_IMPLEMENTED";
  throw err;
}

class Database {
  constructor(_path?: string) {
    notImplemented("sqlite.Database");
  }
}

const sqliteModule = { Database };

export { Database };
export default sqliteModule;
