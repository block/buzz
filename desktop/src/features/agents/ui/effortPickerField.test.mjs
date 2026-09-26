import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { EffortPickerField } from "./EffortPickerField.tsx";

const options = [{ value: "low", displayName: "Low" }];
function render(value, effortOptions = options) {
  return renderToStaticMarkup(
    React.createElement(EffortPickerField, {
      agent: { backend: { type: "local" } },
      config: { effortConfigId: "depth", effortOptions },
      value,
      disabled: false,
      onChange: () =>
        assert.fail("Rendering must not rewrite the saved effort"),
    }),
  );
}

test("the real effort trigger discloses an unavailable explicit value", () => {
  const html = render("high");
  assert.match(html, /high \(unavailable\)/);
  assert.doesNotMatch(html, />Adapter default</);
});

test("catalog recovery restores the adapter label without changing the value", () => {
  const html = render("high", [
    ...options,
    { value: "high", displayName: "High" },
  ]);
  assert.match(html, />High</);
  assert.doesNotMatch(html, /unavailable/);
});

test("only an unset effort displays the adapter default", () => {
  assert.match(render(null), />Adapter default</);
});
