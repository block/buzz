import assert from "node:assert/strict";
import test from "node:test";

import { createChannelPaneAuxiliaryLayout } from "./channelPaneAuxiliaryLayout.ts";

const base = {
  channelManagementOpen: false,
  hasAgentSession: false,
  hasIdleAuxiliaryPanel: true,
  hasIdlePanelCloseHandler: true,
  hasProfilePanel: false,
  hasThreadSurface: false,
  idleAuxiliaryOverridesThread: false,
  isOverlay: false,
  isSinglePanelView: false,
  markdownDocName: "notes.md",
  markdownDocUrl: "http://localhost/media/notes.bin",
  threadViewMode: "focus",
};

test("an open document suppresses idle-drawer coverage when the idle pane yields", () => {
  const layout = createChannelPaneAuxiliaryLayout(base);

  assert.deepEqual(layout.openMarkdownDoc, {
    filename: "notes.md",
    url: "http://localhost/media/notes.bin",
  });
  assert.equal(layout.useFocusIdleDrawer, false);
});

test("an explicitly selected idle pane wins rendering and owns drawer coverage", () => {
  const layout = createChannelPaneAuxiliaryLayout({
    ...base,
    idleAuxiliaryOverridesThread: true,
  });

  assert.equal(layout.openMarkdownDoc, null);
  assert.equal(layout.priorityIdleAuxiliary, true);
  assert.equal(layout.useFocusIdleDrawer, true);
});
