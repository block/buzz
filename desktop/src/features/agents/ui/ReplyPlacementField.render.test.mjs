import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { ReplyPlacementField } from "./ReplyPlacementField.tsx";

function renderPersonaField(value, effectiveValue = "follow-scope") {
  return renderToStaticMarkup(
    React.createElement(ReplyPlacementField, {
      allowInherit: true,
      effectiveValue,
      inheritLabel: "Use global default",
      onChange: () => {},
      value,
    }),
  );
}

test("persona field renders inherited state with the effective global mode", () => {
  const html = renderPersonaField(null);

  assert.match(html, />Use global default</);
  assert.match(html, /Uses the inherited setting \(follow-scope\)/);
  assert.doesNotMatch(html, /Use persona \/ global default/);
});

test("persona field renders an explicit mode after leaving inheritance", () => {
  const html = renderPersonaField("thread");

  assert.match(html, />Always reply in a thread</);
  assert.match(html, /Every human-facing answer uses a thread/);
});

test("persona field shows the historical thread fallback when global is unset", () => {
  const html = renderPersonaField(null, "thread");

  assert.match(html, /Uses the inherited setting \(thread\)/);
  assert.match(html, /historical thread behavior/);
});
