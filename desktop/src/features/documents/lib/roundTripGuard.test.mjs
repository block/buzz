import assert from "node:assert/strict";
import { test } from "node:test";

import {
  classifyRoundTrip,
  initialViewModeFor,
  isRoundTripStable,
} from "./roundTripGuard.ts";

/** A reserializer that returns its input — the "perfectly faithful" case. */
const faithful = (body) => body;

test("identical output is stable", () => {
  assert.equal(isRoundTripStable("# Title\n\nBody.", faithful), true);
});

test("changed output is lossy", () => {
  const mangles = () => "# Different";
  assert.equal(isRoundTripStable("# Title", mangles), false);
});

test("a reserializer that throws is treated as lossy, not stable", () => {
  // Failing open here would autosave a file we could not even parse.
  const explodes = () => {
    throw new Error("parse failure");
  };
  assert.equal(isRoundTripStable("# Title", explodes), false);
});

test("empty and whitespace-only notes are stable without reserializing", () => {
  const explodes = () => {
    throw new Error("must not be called");
  };
  for (const body of ["", "   ", "\n\n", "\t\n "]) {
    assert.equal(isRoundTripStable(body, explodes), true, JSON.stringify(body));
  }
});

test("trailing-newline differences are tolerated", () => {
  assert.equal(
    isRoundTripStable("# Title\n\n", () => "# Title"),
    true,
  );
  assert.equal(
    isRoundTripStable("# Title", () => "# Title\n"),
    true,
  );
});

test("CRLF vs LF is tolerated", () => {
  assert.equal(
    isRoundTripStable("# Title\r\n\r\nBody.", () => "# Title\n\nBody."),
    true,
  );
});

test("interior whitespace changes are NOT tolerated", () => {
  // Collapsing a blank line between paragraphs is a real edit to the file.
  assert.equal(
    isRoundTripStable("para one\n\npara two", () => "para one\npara two"),
    false,
  );
  // Nor is re-indenting a nested list.
  assert.equal(
    isRoundTripStable("- a\n    - b", () => "- a\n  - b"),
    false,
  );
});

test("classifyRoundTrip maps onto the status vocabulary", () => {
  assert.equal(classifyRoundTrip("# Title", faithful), "stable");
  assert.equal(
    classifyRoundTrip("# Title", () => "changed"),
    "lossy",
  );
});

test("only stable notes open in live preview", () => {
  assert.equal(initialViewModeFor("stable"), "live");
  assert.equal(initialViewModeFor("lossy"), "source");
  assert.equal(initialViewModeFor("unknown"), "source");
});
