import assert from "node:assert/strict";
import test from "node:test";

import {
  describeChannelTextFieldChange,
  toInlineName,
} from "./systemEventCopy.ts";

test("a set topic is quoted verbatim", () => {
  assert.equal(
    describeChannelTextFieldChange("topic", "Release planning"),
    "changed the topic to “Release planning”",
  );
});

test("a set purpose names the purpose, not the topic", () => {
  assert.equal(
    describeChannelTextFieldChange("purpose", "Where we ship from"),
    "changed the purpose to “Where we ship from”",
  );
});

// The relay reports a clear as a change carrying an empty string, so without
// this branch the timeline reads: changed the topic to “”.
test("an empty value reads as cleared, not as a change to empty quotes", () => {
  for (const blank of ["", undefined, null]) {
    assert.equal(
      describeChannelTextFieldChange("topic", blank),
      "cleared the channel topic",
    );
    assert.equal(
      describeChannelTextFieldChange("purpose", blank),
      "cleared the channel purpose",
    );
  }
});

test("a whitespace-only value reads as cleared", () => {
  assert.equal(
    describeChannelTextFieldChange("topic", "   \n\t "),
    "cleared the channel topic",
  );
});

test("surrounding whitespace is trimmed out of the quotes", () => {
  assert.equal(
    describeChannelTextFieldChange("topic", "  Release planning  "),
    "changed the topic to “Release planning”",
  );
});

test("no caption announces empty quotes", () => {
  for (const value of ["", " ", null, undefined, "Real topic"]) {
    for (const field of ["topic", "purpose"]) {
      assert.doesNotMatch(
        describeChannelTextFieldChange(field, value),
        /“”|""/,
        `${field} with ${JSON.stringify(value)} must not render empty quotes`,
      );
    }
  }
});

test("the current user is lowercase mid-sentence", () => {
  // "added by You" next to an agent's "managed by you" was the inconsistency.
  assert.equal(toInlineName("You"), "you");
});

test("every other name keeps its own capitalization", () => {
  for (const name of [
    "Alice Chen",
    "you-know-who",
    "Someone",
    "npub1abc…def",
  ]) {
    assert.equal(toInlineName(name), name);
  }
});

test("only the exact self label is rewritten", () => {
  // A person really named "Your Highness" or "Youssef" is not the current user.
  assert.equal(toInlineName("Youssef"), "Youssef");
  assert.equal(toInlineName("You Know Who"), "You Know Who");
});
