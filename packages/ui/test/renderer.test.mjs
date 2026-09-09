import assert from "node:assert/strict";
import { test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { GeneratedResponse } from "../dist/index.js";
const response = {
  version: 1,
  title: "Result",
  blocks: [
    { id: "text", type: "text", text: "<script>alert(1)</script>" },
    {
      id: "actions",
      type: "actions",
      items: [{ id: "review", label: "Review" }],
    },
  ],
};
const render = (props) =>
  renderToStaticMarkup(
    createElement(GeneratedResponse, { response, ...props }),
  );
test("renderer escapes generated markup and does not dispatch actions during rendering", () => {
  let called = 0;
  const html = render({ actions: { review: () => called++ } });
  assert.equal(called, 0);
  assert.ok(html.includes("&lt;script&gt;"));
  assert.ok(!html.includes("<script>"));
  assert.ok(!html.includes('disabled=""'));
});
test("unregistered, inherited, streaming, and pending actions are disabled", () => {
  for (const props of [
    {},
    { actions: Object.create({ review: () => {} }) },
    { actions: { review: () => {} }, state: "streaming" },
    { actions: { review: () => {} }, pendingAction: "review" },
  ])
    assert.ok(render(props).includes('disabled=""'));
});
test("invalid responses render a recovery action supplied by the host", () => {
  const html = render({ response: { version: 2 }, onRetry: () => {} });
  assert.ok(html.includes("This response could not be displayed"));
  assert.ok(html.includes("Try again"));
});
