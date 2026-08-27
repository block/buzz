import assert from "node:assert/strict";
import test from "node:test";

import {
  requestCoverDrawerClose,
  subscribeToCoverDrawerCloseRequest,
} from "./coverDrawerCloseRequest.ts";

test("cover drawer close requests reach active subscribers only", () => {
  let calls = 0;
  const unsubscribe = subscribeToCoverDrawerCloseRequest(() => {
    calls += 1;
  });

  requestCoverDrawerClose();
  assert.equal(calls, 1);

  unsubscribe();
  requestCoverDrawerClose();
  assert.equal(calls, 1);
});
