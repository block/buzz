import assert from "node:assert/strict";
import test from "node:test";

import {
  buildNotifyTags,
  detectNotifyMode,
  isReservedMentionName,
  reservedMentionToken,
} from "./channelNotify.ts";

test("reserved tokens are matched exactly and case-insensitively", () => {
  assert.equal(reservedMentionToken("channel"), "channel");
  assert.equal(reservedMentionToken("Here"), "here");
  assert.equal(reservedMentionToken("  CHANNEL "), "channel");
  assert.equal(reservedMentionToken("channels"), null);
  assert.equal(reservedMentionToken("hereford"), null);
  assert.equal(isReservedMentionName("HERE"), true);
  assert.equal(isReservedMentionName("Herelia"), false);
});

test("detectNotifyMode finds either mode in ordinary prose", () => {
  assert.equal(detectNotifyMode("heads up @channel"), "channel");
  assert.equal(detectNotifyMode("@here can someone look?"), "here");
  assert.equal(detectNotifyMode("**@here** please"), "here");
  assert.equal(detectNotifyMode("no mention at all"), null);
  assert.equal(detectNotifyMode("mail me at foo@here.example"), null);
});

test("detectNotifyMode prefers @channel when both appear", () => {
  assert.equal(detectNotifyMode("@here and @channel"), "channel");
  assert.equal(detectNotifyMode("@channel plus @here"), "channel");
});

test("detectNotifyMode ignores tokens inside code", () => {
  assert.equal(detectNotifyMode("use `@here` in a message"), null);
  assert.equal(detectNotifyMode("```\n@channel\n```"), null);
  assert.equal(detectNotifyMode("    @channel"), null);
  // A real mention alongside a code sample still notifies.
  assert.equal(detectNotifyMode("@channel see `@here`"), "channel");
});

test("buildNotifyTags emits at most one marker tag", () => {
  assert.deepEqual(buildNotifyTags("channel"), [["notify", "channel"]]);
  assert.deepEqual(buildNotifyTags("here"), [["notify", "here"]]);
  assert.deepEqual(buildNotifyTags(null), []);
});
