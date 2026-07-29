import assert from "node:assert/strict";
import test from "node:test";

import { getScreenLayout } from "./threadPanelLayout.ts";

test("uses single-panel layout for requested auxiliary content on narrow channels", () => {
  assert.deepEqual(
    getScreenLayout({
      surfaceActive: false,
      auxiliaryPanelRequested: true,
      channelType: "stream",
      contentWidthPx: 500,
    }),
    {
      isSinglePanelView: true,
      shouldCompactHeaderActions: true,
    },
  );
});

test("gives an active channel app the full content layout", () => {
  assert.deepEqual(
    getScreenLayout({
      surfaceActive: true,
      auxiliaryPanelRequested: true,
      channelType: "stream",
      contentWidthPx: 700,
    }),
    {
      isSinglePanelView: false,
      shouldCompactHeaderActions: false,
    },
  );
});
