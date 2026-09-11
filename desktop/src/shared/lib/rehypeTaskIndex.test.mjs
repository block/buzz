import assert from "node:assert/strict";
import { test } from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { readTaskIndex, rehypeTaskIndex } from "./rehypeTaskIndex.ts";
import { countTaskMarkers, toggleTaskMarker } from "./toggleTaskMarker.mjs";

/**
 * Render `source` through the same plugin pair the app uses and report every
 * task checkbox the renderer actually produced, in stamped order.
 *
 * @param {string} source
 * @returns {{ index: number | null, checked: boolean }[]}
 */
function renderCheckboxes(source) {
  const seen = [];
  const components = {
    input: (props) => {
      if (props.type !== "checkbox") return null;
      seen.push({
        index: readTaskIndex(props["data-task-index"]),
        checked: Boolean(props.checked),
      });
      return null;
    },
  };
  renderToStaticMarkup(
    React.createElement(
      Markdown,
      {
        remarkPlugins: [remarkGfm],
        rehypePlugins: [rehypeTaskIndex],
        components,
      },
      source,
    ),
  );
  return seen;
}

test("stamps a dense, gapless ordinal in document order", () => {
  const boxes = renderCheckboxes("- [ ] a\n- [x] b\n- [ ] c");
  assert.deepEqual(
    boxes.map((b) => b.index),
    [0, 1, 2],
  );
  assert.deepEqual(
    boxes.map((b) => b.checked),
    [false, true, false],
  );
});

test("the source counter agrees with the renderer on tricky input", () => {
  // This is the invariant the whole feature rests on: if these two ever
  // disagree, a click toggles the wrong line. Fenced samples, a bracket with
  // no trailing space, and a nested list are the cases that break naive
  // counting.
  const source = [
    "Pendientes:",
    "",
    "```",
    "- [ ] esto es un ejemplo en código",
    "```",
    "",
    "- [ ] revisar relay",
    "- [ ]sin-espacio-no-es-tarea",
    "- [x] cerrar incidente",
    "  - [ ] subtarea anidada",
  ].join("\n");

  const boxes = renderCheckboxes(source);
  assert.equal(boxes.length, countTaskMarkers(source));
  assert.deepEqual(
    boxes.map((b) => b.index),
    [0, 1, 2],
  );
});

test("toggling ordinal N flips exactly the checkbox the renderer numbered N", () => {
  const source = [
    "```",
    "- [ ] señuelo en código",
    "```",
    "- [ ] uno",
    "- [ ] dos",
    "- [ ] tres",
  ].join("\n");

  for (const target of [0, 1, 2]) {
    const next = toggleTaskMarker(source, target, true);
    assert.notEqual(next, null, `ordinal ${target} debió resolver`);
    const boxes = renderCheckboxes(next);
    assert.deepEqual(
      boxes.map((b) => b.checked),
      [0, 1, 2].map((i) => i === target),
      `solo la casilla ${target} debió quedar palomeada`,
    );
  }
});

test("a round trip through check and uncheck restores the original", () => {
  const source = "- [ ] uno\n- [x] dos";
  const checked = toggleTaskMarker(source, 0, true);
  assert.equal(checked, "- [x] uno\n- [x] dos");
  assert.equal(toggleTaskMarker(checked, 0, false), source);
});

test("readTaskIndex refuses anything that is not a plain ordinal", () => {
  assert.equal(readTaskIndex("0"), 0);
  assert.equal(readTaskIndex("12"), 12);
  assert.equal(readTaskIndex(undefined), null);
  assert.equal(readTaskIndex("-1"), null);
  assert.equal(readTaskIndex("1.5"), null);
  assert.equal(readTaskIndex("1e3"), null);
  assert.equal(readTaskIndex(3), null);
});
