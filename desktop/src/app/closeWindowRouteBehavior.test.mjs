import assert from "node:assert/strict";
import test from "node:test";

import { shouldCmdWCloseWindowForRoute } from "./closeWindowRouteBehavior.ts";

test("Cmd+W closes the window on routes without an active chat selection", () => {
  assert.equal(shouldCmdWCloseWindowForRoute({ pathname: "/" }), true);
  assert.equal(shouldCmdWCloseWindowForRoute({ pathname: "/agents" }), true);
});

test("Cmd+W is reserved for clearing active chat selections", () => {
  assert.equal(
    shouldCmdWCloseWindowForRoute({ pathname: "/channels/general" }),
    false,
  );
  assert.equal(
    shouldCmdWCloseWindowForRoute({ pathname: "/messages/new" }),
    false,
  );
});

test("home detail selections are treated as active chats", () => {
  assert.equal(
    shouldCmdWCloseWindowForRoute({
      pathname: "/",
      search: { item: "event-1" },
    }),
    false,
  );
  assert.equal(
    shouldCmdWCloseWindowForRoute({ pathname: "/", search: { item: "" } }),
    true,
  );
});
