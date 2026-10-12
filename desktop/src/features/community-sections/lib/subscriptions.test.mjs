import assert from "node:assert/strict";
import test from "node:test";

import {
  EMPTY_SUBSCRIPTION_STORE,
  isCommunitySectionSubscribed,
  parseCommunitySectionSubsPayload,
  setCommunitySectionSubscribed,
  subscribedCommunitySectionIds,
} from "./subscriptions.ts";

test("parseCommunitySectionSubsPayload accepts v1 store", () => {
  const store = parseCommunitySectionSubsPayload({
    version: 1,
    subscribed: { eng: true, ops: false },
  });
  assert.equal(store?.subscribed.eng, true);
  assert.equal(store?.subscribed.ops, false);
});

test("setCommunitySectionSubscribed toggles membership", () => {
  let store = EMPTY_SUBSCRIPTION_STORE;
  store = setCommunitySectionSubscribed(store, "eng", true);
  assert.equal(isCommunitySectionSubscribed(store, "eng"), true);
  assert.deepEqual(subscribedCommunitySectionIds(store), ["eng"]);
  store = setCommunitySectionSubscribed(store, "eng", false);
  assert.equal(isCommunitySectionSubscribed(store, "eng"), false);
  assert.deepEqual(subscribedCommunitySectionIds(store), []);
});

test("web default: sections are subscribed unless explicitly turned off", () => {
  let store = EMPTY_SUBSCRIPTION_STORE;
  assert.equal(isCommunitySectionSubscribed(store, "eng", true), true);
  store = setCommunitySectionSubscribed(store, "eng", false, true);
  assert.equal(store.subscribed.eng, false);
  assert.equal(isCommunitySectionSubscribed(store, "eng", true), false);
  store = setCommunitySectionSubscribed(store, "eng", true, true);
  assert.equal(isCommunitySectionSubscribed(store, "eng", true), true);
});
