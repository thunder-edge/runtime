type NodeLikeError = Error & { code?: string };

function notImplemented(api: string): never {
  const err = new Error(
    `[thunder] ${api} is not implemented in this runtime profile`,
  ) as NodeLikeError;
  err.code = "ERR_NOT_IMPLEMENTED";
  throw err;
}

function test(): never {
  return notImplemented("node:test");
}

const it = test;
const describe = test;
const before = test;
const after = test;
const beforeEach = test;
const afterEach = test;

const testModule = {
  test,
  it,
  describe,
  before,
  after,
  beforeEach,
  afterEach,
};

export { test, it, describe, before, after, beforeEach, afterEach };
export default testModule;
