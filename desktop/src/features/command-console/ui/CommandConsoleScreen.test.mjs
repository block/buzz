import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CommandConsoleScreen } from "./CommandConsoleScreen.tsx";

const ADVISERS = [
  "Chief of Staff",
  "Operations",
  "Navigation",
  "Daily Routine",
  "Reporting",
  "Plans",
];

test("CommandConsoleScreen renders an unmistakable OFFICIAL classification", () => {
  const html = renderToStaticMarkup(React.createElement(CommandConsoleScreen));

  assert.match(html, /data-testid="command-console-screen"/);
  assert.match(html, /data-testid="command-console-official-banner"/);
  assert.match(html, />OFFICIAL</);
});

test("CommandConsoleScreen marks all six adviser placeholders as not yet operational", () => {
  const html = renderToStaticMarkup(React.createElement(CommandConsoleScreen));

  for (const adviser of ADVISERS) {
    assert.match(html, new RegExp(`>${adviser}<`));
  }

  assert.equal(html.match(/>Not yet operational</g)?.length, ADVISERS.length);
});
