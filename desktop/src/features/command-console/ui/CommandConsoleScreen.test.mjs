import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CommandConsoleScreen } from "./CommandConsoleScreen.tsx";
import { CommandTeamStripView } from "./CommandTeamStrip.tsx";

function renderCommandConsole() {
  return renderToStaticMarkup(
    React.createElement(CommandConsoleScreen, {
      commandTeam: React.createElement(CommandTeamStripView, {
        error: null,
        onMessage: () => {},
        pendingPersonaIds: new Set(),
      }),
    }),
  );
}

test("CommandConsoleScreen renders the usable Command Adviser route", () => {
  const html = renderCommandConsole();

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
    "intelligence",
    "logistics",
    "navigation",
    "daily-routine",
    "reporting",
    "plans",
  ]) {
    assert.match(html, new RegExp(`data-testid="adviser-insignia-${adviser}"`));
  }
  assert.equal((html.match(/>Message</g) ?? []).length, 8);
  assert.doesNotMatch(html, />Command Console</);
  assert.doesNotMatch(html, /unsigned|fingerprint|replication/i);
});

test("CommandConsoleScreen installs the real advisory Daily Command Brief without not-operational claims", () => {
  const html = renderCommandConsole();

  assert.match(html, /data-testid="daily-command-brief"/);
  assert.match(html, />Daily Command Brief</);
  assert.match(html, /Advisory, non-accredited decision support/);
  assert.doesNotMatch(html, /not yet operational/i);
});
