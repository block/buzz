import assert from "node:assert/strict";
import test from "node:test";

import {
  canonicalChannelName,
  channelNamesMatch,
} from "./canonicalChannelName.ts";

test("canonicalChannelName strips interleaved leading hashes and whitespace", () => {
  assert.equal(canonicalChannelName("channel"), "channel");
  assert.equal(canonicalChannelName("#channel"), "channel");
  assert.equal(canonicalChannelName("  ### channel  "), "channel");
  assert.equal(canonicalChannelName("# #"), "");
  assert.equal(canonicalChannelName("### ###"), "");
  assert.equal(canonicalChannelName("channel#topic"), "channel#topic");
});

test("channelNamesMatch canonicalizes both legacy names and search input", () => {
  assert.equal(channelNamesMatch("#general", "general"), true);
  assert.equal(channelNamesMatch("general", " #GENERAL "), true);
  assert.equal(channelNamesMatch("#random", "general"), false);
});
