import assert from "node:assert/strict";
import test from "node:test";
import { phoneHidesTabBar, phoneTabForPath } from "./phoneTabs.ts";

test("routes map to tabs (deep links)", () => {
  assert.equal(phoneTabForPath("/", null), "inbox");
  assert.equal(phoneTabForPath("/channels/abc", null), "channels");
  assert.equal(phoneTabForPath("/channels/abc/posts/def", null), "channels");
  assert.equal(phoneTabForPath("/agents", null), "me");
  assert.equal(phoneTabForPath("/buzz-term", null), "term");
  assert.equal(phoneTabForPath("/bots/x", null), "me");
  assert.equal(phoneTabForPath("/settings", null), "me");
  assert.equal(phoneTabForPath("/agents", "channels"), "channels");
  assert.equal(phoneTabForPath("/", "me"), "me");
  assert.equal(phoneTabForPath("/channels/abc", "search"), "search");
});

test("conversation views hide the tab bar; top-level screens show it", () => {
  assert.equal(phoneHidesTabBar("/channels/abc", null), true);
  assert.equal(phoneHidesTabBar("/channels/abc/posts/def", null), true);
  assert.equal(phoneHidesTabBar("/", null), false);
  assert.equal(phoneHidesTabBar("/agents", null), false);
  assert.equal(phoneHidesTabBar("/channels/abc", "channels"), false);
});
