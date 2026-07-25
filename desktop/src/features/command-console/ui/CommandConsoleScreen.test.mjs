import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CommandConsoleScreen } from "./CommandConsoleScreen.tsx";

test("CommandConsoleScreen renders an unmistakable OFFICIAL classification", () => {
  const html = renderToStaticMarkup(React.createElement(CommandConsoleScreen));

  assert.match(html, /data-testid="command-console-screen"/);
  assert.match(html, /data-testid="command-console-official-banner"/);
  assert.match(html, />OFFICIAL</);
});

test("CommandConsoleScreen installs the real advisory Daily Command Brief without placeholder claims", () => {
  const html = renderToStaticMarkup(React.createElement(CommandConsoleScreen));

  assert.match(html, /data-testid="daily-command-brief"/);
  assert.match(html, />Daily Command Brief</);
  assert.match(html, /Advisory, non-accredited decision support/);
  assert.doesNotMatch(html, /placeholder|not yet operational/i);
});
