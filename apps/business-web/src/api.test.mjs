import assert from "node:assert/strict";
import test from "node:test";
import { apiErrorMessage } from "./api.ts";

test("localizes the deliberately opaque authorization miss", () => {
  assert.equal(
    apiErrorMessage(
      {
        code: "not_found_or_forbidden",
        message: "resource was not found or is not accessible",
      },
      404,
    ),
    "资源不存在，或当前账号无权访问",
  );
});

test("preserves a specific server message", () => {
  assert.equal(
    apiErrorMessage({ code: "invalid_request", message: "订单版本已变化" }, 409),
    "订单版本已变化",
  );
});

test("falls back to the response status for an empty body", () => {
  assert.equal(apiErrorMessage({}, 503), "请求失败（503）");
});
