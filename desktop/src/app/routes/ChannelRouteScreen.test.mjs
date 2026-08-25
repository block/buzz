import assert from "node:assert/strict";
import test from "node:test";

import {
  getValidatedRouteThreadRootId,
  hasValidRouteThreadIntent,
  isRouteEventForChannel,
} from "./ChannelRouteScreen.tsx";

function event(id, tags = [["h", "channel"]]) {
  return {
    id,
    pubkey: "author",
    created_at: 1,
    kind: 9,
    tags,
    content: "hello",
    sig: "signature",
  };
}

test("a top-level route only accepts its own id as thread root", () => {
  const target = event("target");
  assert.equal(getValidatedRouteThreadRootId(target, "target"), "target");
  assert.equal(getValidatedRouteThreadRootId(target, "unrelated"), null);
  assert.equal(getValidatedRouteThreadRootId(target, null), null);
});

test("a reply route derives its containing root", () => {
  const target = event("reply", [
    ["h", "channel"],
    ["e", "root", "", "root"],
    ["e", "root", "", "reply"],
  ]);
  assert.equal(getValidatedRouteThreadRootId(target, null), "root");
  assert.equal(getValidatedRouteThreadRootId(target, "root"), "root");
  assert.equal(getValidatedRouteThreadRootId(target, "unrelated-root"), null);
  assert.equal(hasValidRouteThreadIntent(target, null), true);
  assert.equal(hasValidRouteThreadIntent(target, "root"), true);
  assert.equal(hasValidRouteThreadIntent(target, "unrelated-root"), false);
});

test("route events must belong to the routed channel", () => {
  assert.equal(isRouteEventForChannel(event("target"), "channel"), true);
  assert.equal(isRouteEventForChannel(event("target"), "other-channel"), false);
  assert.equal(isRouteEventForChannel(event("target", []), "channel"), false);
});
