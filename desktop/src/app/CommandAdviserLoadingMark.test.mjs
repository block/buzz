import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CommandAdviserLoadingMark } from "./CommandAdviserLoadingMark.tsx";

test("startup mark presents Command Adviser without Buzz branding", () => {
  const html = renderToStaticMarkup(
    React.createElement(CommandAdviserLoadingMark),
  );

  assert.match(html, /Command Adviser/);
  assert.match(html, /Strengthen the Shield/i);
  assert.doesNotMatch(html, />[^<]*(?:Buzz|bee)[^<]*</i);
  assert.doesNotMatch(html, /aria-label="[^"]*(?:Buzz|bee)/i);
});
