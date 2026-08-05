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

test("soft-wrapped prose is tolerated", () => {
  // Hard-wrapping at ~80 columns is ubiquitous, and a single newline inside a
  // paragraph is a soft break the serializer legitimately joins. Treating that
  // as lossy pushed almost every real note into source mode.
  const wrapped = "This guide is for agents. It covers\nconventions and setup.";
  const joined = "This guide is for agents. It covers conventions and setup.";
  assert.equal(
    isRoundTripStable(wrapped, () => joined),
    true,
  );
});

test("soft-wrap tolerance does not mask real losses", () => {
  // A dropped link, escaped HTML, or a destroyed table must still be caught
  // even though they also change line structure.
  assert.equal(
    isRoundTripStable("A [`hash`](https://x)", () => "A `hash`"),
    false,
  );
  assert.equal(
    isRoundTripStable(
      '<h1 align="center">T</h1>',
      () => "&lt;h1&gt;T&lt;/h1&gt;",
    ),
    false,
  );
  assert.equal(
    isRoundTripStable("| a | b |\n| --- | --- |\n| 1 | 2 |", () => "ab12"),
    false,
  );
});

test("block starts are never joined into the previous line", () => {
  // A list following a paragraph must stay a list, not be absorbed into it.
  assert.equal(
    isRoundTripStable("Intro text\n- item one\n- item two", () => "joined"),
    false,
  );
  // An explicit two-space hard break is preserved, not treated as soft.
  assert.equal(
    isRoundTripStable("line one  \nline two", () => "line one line two"),
    false,
  );
});

test("fenced code keeps its line structure", () => {
  // Joining lines inside a fence would hide real corruption of code blocks.
  const code = "```js\nconst a = 1;\nconst b = 2;\n```";
  assert.equal(
    isRoundTripStable(code, () => "```js\nconst a = 1; const b = 2;\n```"),
    false,
  );
});

test("table delimiter dash counts are cosmetic", () => {
  // GFM ignores delimiter width, and the serializer emits a canonical three
  // dashes, so a hand-aligned source row must not read as lossy.
  assert.equal(
    isRoundTripStable(
      "| a | b |\n| ------- | --------- |\n| 1 | 2 |",
      () => "| a | b |\n| --- | --- |\n| 1 | 2 |",
    ),
    true,
  );
});

test("alignment colons in a delimiter row are preserved", () => {
  // Colons change column alignment, so losing one is a real change.
  assert.equal(
    isRoundTripStable("| a |\n| :--- |\n| 1 |", () => "| a |\n| --- |\n| 1 |"),
    false,
  );
});
