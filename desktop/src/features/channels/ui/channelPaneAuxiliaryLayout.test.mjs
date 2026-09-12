import assert from "node:assert/strict";
import test from "node:test";

import { createChannelPaneAuxiliaryLayout } from "./channelPaneAuxiliaryLayout.ts";

const base = {
  canFitThirdPanel: false,
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

test("an open document stacks over an existing split thread", () => {
  const layout = createChannelPaneAuxiliaryLayout({
    ...base,
    hasThreadSurface: true,
  });

  assert.equal(layout.useStackedMarkdownPanel, true);
  assert.deepEqual(layout.openMarkdownDoc, {
    filename: "notes.md",
    url: "http://localhost/media/notes.bin",
  });
});

test("a wide layout shows the document beside the existing thread", () => {
  const layout = createChannelPaneAuxiliaryLayout({
    ...base,
    canFitThirdPanel: true,
    hasThreadSurface: true,
  });

  assert.equal(layout.showMarkdownBesideThread, true);
  assert.equal(layout.useStackedMarkdownPanel, false);
});

test("a document without a thread remains in the ordinary auxiliary pane", () => {
  const layout = createChannelPaneAuxiliaryLayout(base);
  assert.equal(layout.useStackedMarkdownPanel, false);
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
