import assert from "node:assert/strict";
import test from "node:test";

import {
  requestFocusedThreadClose,
  subscribeToFocusedThreadCloseRequest,
} from "./focusedThreadCloseRequest.ts";

test("focus thread close requests reach active subscribers only", () => {
  let calls = 0;
  const unsubscribe = subscribeToFocusedThreadCloseRequest(() => {
    calls += 1;
  });

  requestFocusedThreadClose();
  assert.equal(calls, 1);

  unsubscribe();
  requestFocusedThreadClose();
  assert.equal(calls, 1);
});
