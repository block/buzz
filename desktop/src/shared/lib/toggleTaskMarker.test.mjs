import assert from "node:assert/strict";
import { test } from "node:test";

import { countTaskMarkers, toggleTaskMarker } from "./toggleTaskMarker.mjs";

test("checks the addressed task and leaves its siblings alone", () => {
  const source = "- [ ] uno\n- [ ] dos\n- [ ] tres";
  assert.equal(
    toggleTaskMarker(source, 1, true),
    "- [ ] uno\n- [x] dos\n- [ ] tres",
  );
});

test("unchecks an already-checked task", () => {
  assert.equal(toggleTaskMarker("- [x] hecho", 0, false), "- [ ] hecho");
});

test("preserves indentation, bullet style and trailing text", () => {
  const source = "  * [ ]   revisar   \n1. [ ] segundo";
  assert.equal(
    toggleTaskMarker(source, 0, true),
    "  * [x]   revisar   \n1. [ ] segundo",
  );
  assert.equal(
    toggleTaskMarker(source, 1, true),
    "  * [ ]   revisar   \n1. [x] segundo",
  );
});

test("skips markers inside fenced code, which render as code not checkboxes", () => {
  // The renderer emits exactly one checkbox here, so ordinal 0 must land on
  // the real task below the fence — not on the sample inside it.
  const source = ["```", "- [ ] ejemplo", "```", "- [ ] real"].join("\n");
  assert.equal(countTaskMarkers(source), 1);
  assert.equal(
    toggleTaskMarker(source, 0, true),
    ["```", "- [ ] ejemplo", "```", "- [x] real"].join("\n"),
  );
});

test("handles tilde fences and ignores a mismatched fence character", () => {
  const source = ["~~~", "- [ ] dentro", "~~~", "- [ ] fuera"].join("\n");
  assert.equal(countTaskMarkers(source), 1);
});

test("a bare bracket with no space after it is not a task marker", () => {
  // GFM requires whitespace (or end of line) after the closing bracket.
  assert.equal(countTaskMarkers("- [ ]sin-espacio"), 0);
  assert.equal(toggleTaskMarker("- [ ]sin-espacio", 0, true), null);
});

test("returns null when the ordinal resolves to nothing", () => {
  // Guards the concurrent-edit case: the message changed between render and
  // click, so there is no safe write.
  assert.equal(toggleTaskMarker("- [ ] solo una", 3, true), null);
  assert.equal(toggleTaskMarker("sin tareas", 0, true), null);
});

test("rejects malformed ordinals instead of writing something arbitrary", () => {
  assert.equal(toggleTaskMarker("- [ ] uno", -1, true), null);
  assert.equal(toggleTaskMarker("- [ ] uno", 1.5, true), null);
  assert.equal(toggleTaskMarker("- [ ] uno", Number.NaN, true), null);
});

test("counts across mixed prose and lists", () => {
  const source = "texto\n\n- [ ] a\n- [x] b\n\notro\n\n1) [ ] c";
  assert.equal(countTaskMarkers(source), 3);
  assert.equal(
    toggleTaskMarker(source, 2, true),
    "texto\n\n- [ ] a\n- [x] b\n\notro\n\n1) [x] c",
  );
});
