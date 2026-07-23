import assert from "node:assert/strict";
import test from "node:test";

import { canonicalChannelName } from "./canonicalChannelName.ts";

test("canonicalChannelName strips interleaved leading hashes and whitespace", () => {
  assert.equal(canonicalChannelName("channel"), "channel");
  assert.equal(canonicalChannelName("#channel"), "channel");
  assert.equal(canonicalChannelName("  ### channel  "), "channel");
  assert.equal(canonicalChannelName("# #"), "");
  assert.equal(canonicalChannelName("### ###"), "");
  assert.equal(canonicalChannelName("channel#topic"), "channel#topic");
});
