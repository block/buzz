import assert from "node:assert/strict";
import test from "node:test";

import { getAgentSessionPanelPresentation } from "./agentSessionPanelPresentation.ts";

test("the cover drawer owns motion and gets standalone, opaque chrome", () => {
  assert.deepEqual(
    getAgentSessionPanelPresentation({
      isCoverDrawer: true,
      isSinglePanelView: false,
      useSplitAuxiliaryPane: true,
    }),
    {
      enterMotion: false,
      isSinglePanelView: true,
      layout: "standalone",
      transparentChrome: false,
    },
  );
});

test("the split pane keeps docked chrome and its own enter motion", () => {
  assert.deepEqual(
    getAgentSessionPanelPresentation({
      isCoverDrawer: false,
      isSinglePanelView: false,
      useSplitAuxiliaryPane: true,
    }),
    {
      enterMotion: true,
      isSinglePanelView: false,
      layout: "split",
      transparentChrome: true,
    },
  );
});

test("narrow viewports keep today's overlay and single-panel presentations", () => {
  for (const isSinglePanelView of [false, true]) {
    assert.deepEqual(
      getAgentSessionPanelPresentation({
        isCoverDrawer: false,
        isSinglePanelView,
        useSplitAuxiliaryPane: false,
      }),
      {
        enterMotion: true,
        isSinglePanelView,
        layout: "standalone",
        transparentChrome: false,
      },
    );
  }
});
