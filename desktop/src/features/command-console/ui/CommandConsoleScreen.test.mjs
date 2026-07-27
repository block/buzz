import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CommandConsoleScreen } from "./CommandConsoleScreen.tsx";

test("CommandConsoleScreen renders the usable Command Adviser route", () => {
  const html = renderToStaticMarkup(React.createElement(CommandConsoleScreen));

  assert.match(html, /data-testid="command-console-screen"/);
  assert.match(html, /data-testid="command-console-official-banner"/);
  assert.match(html, /data-testid="model-routing-controls"/);
  assert.match(html, />COMMAND ADVISER</);
  assert.match(html, /Cloud models first/i);
  assert.match(html, /Local model first/i);
  assert.match(html, /HMAS SUPPLY · A195/);
  assert.match(html, /STRENGTHEN THE SHIELD/);
  assert.match(html, /alt="HMAS Supply at sea"/);
  for (const adviser of [
    "chief-of-staff",
    "operations",
    "navigation",
    "daily-routine",
    "reporting",
    "plans",
  ]) {
    assert.match(html, new RegExp(`data-testid="adviser-insignia-${adviser}"`));
  }
  assert.doesNotMatch(html, />Command Console</);
  assert.doesNotMatch(html, /unsigned|fingerprint|replication/i);
});

test("CommandConsoleScreen installs the real advisory Daily Command Brief without placeholder claims", () => {
  const html = renderToStaticMarkup(React.createElement(CommandConsoleScreen));

  assert.match(html, /data-testid="daily-command-brief"/);
  assert.match(html, />Daily Command Brief</);
  assert.match(html, /Advisory, non-accredited decision support/);
  assert.doesNotMatch(html, /placeholder|not yet operational/i);
});
