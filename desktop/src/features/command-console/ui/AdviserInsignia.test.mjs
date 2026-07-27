import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  COMMAND_TEAM_PERSONAS,
  commandAdviserForPersona,
  isCommandTeamPersonaId,
} from "../domain/commandTeam.ts";
import { AdviserInsignia } from "./AdviserInsignia.tsx";

const EXPECTED = {
  chief_of_staff: "Chief of Staff — command anchor",
  operations: "Operations Adviser — radar plot",
  navigation: "Navigation Adviser — sextant",
  daily_routine: "Daily Routine Adviser — ship's bell",
  reporting: "Reporting Adviser — clipboard and returns",
  plans: "Plans Adviser — charted course",
};

test("renders six distinct accessible naval adviser symbols", () => {
  const symbols = new Set();

  for (const [adviser, label] of Object.entries(EXPECTED)) {
    const html = renderToStaticMarkup(
      React.createElement(AdviserInsignia, { adviser }),
    );
    const serializedLabel = label.replaceAll("'", "&#x27;");
    assert.match(html, new RegExp(`aria-label="${serializedLabel}"`));
    const symbol = html.match(/data-symbol="([^"]+)"/)?.[1];
    assert.ok(symbol, `missing data-symbol for ${adviser}`);
    symbols.add(symbol);
  }

  assert.equal(symbols.size, 6);
});

test("maps each stable command persona to one adviser identity", () => {
  assert.deepEqual(
    COMMAND_TEAM_PERSONAS.map(({ adviser, personaId }) => [adviser, personaId]),
    [
      ["chief_of_staff", "builtin:command-chief-of-staff"],
      ["operations", "builtin:command-operations"],
      ["navigation", "builtin:command-navigation"],
      ["daily_routine", "builtin:command-daily-routine"],
      ["reporting", "builtin:command-reporting"],
      ["plans", "builtin:command-plans"],
    ],
  );

  for (const { adviser, personaId } of COMMAND_TEAM_PERSONAS) {
    assert.equal(isCommandTeamPersonaId(personaId), true);
    assert.equal(commandAdviserForPersona(personaId), adviser);
  }
  assert.equal(isCommandTeamPersonaId("builtin:fizz"), false);
  assert.equal(commandAdviserForPersona("builtin:fizz"), undefined);
});
