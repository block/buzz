import assert from "node:assert/strict";
import test from "node:test";

import {
  isThreadRailVisible,
  projectThreadRailLayout,
  threadRailColumnClassName,
  threadRailEntryClassName,
  threadRailHeaderClassName,
  threadRailShellClassName,
} from "./threadRailLayout.ts";

test("isThreadRailVisible hides the rail at and below the desktop breakpoint", () => {
  assert.equal(isThreadRailVisible(600, 1), false);
  assert.equal(isThreadRailVisible(599, 1), false);
  assert.equal(isThreadRailVisible(601, 1), true);
  assert.equal(isThreadRailVisible(601, 0), false);
});

test("Thread Rail column replaces exposed sidebar paint without adding corners", () => {
  const className = threadRailColumnClassName();

  assert.match(className, /buzz-theme-gradient-underlay/);
  assert.match(className, /self-stretch/);
  assert.match(className, /shrink-0/);
  assert.doesNotMatch(className, /rounded/);
  assert.doesNotMatch(className, /bg-background/);
  assert.doesNotMatch(className, /bg-sidebar/);
  assert.doesNotMatch(className, /shadow/);
  assert.doesNotMatch(className, /ring/);
});

test("Thread Rail paints one full-height rounded panel inside the unframed column", () => {
  for (const collapsed of [false, true]) {
    const className = threadRailShellClassName(collapsed);

    assert.match(className, /mt-px/);
    assert.match(className, /mb-2/);
    assert.match(className, /(?<!max-)h-\[calc\(100%-0\.5625rem\)\]/);
    assert.match(className, /self-start/);
    assert.match(className, /rounded-2xl/);
    assert.match(className, /overflow-hidden/);
    assert.match(className, /bg-background(?!\/)/);
    assert.match(className, /shadow-content-edge/);
    assert.match(className, /ring-border\/30/);
    assert.match(className, /ring-inset/);
    assert.doesNotMatch(className, /max-h-\[calc\(100%-0\.5625rem\)\]/);
    assert.doesNotMatch(className, /clip-path/);
    assert.doesNotMatch(className, /self-stretch/);
    assert.doesNotMatch(className, /\bmy-2\b/);
  }
});

test("Thread Rail header and rows use native panel density and one full-width selection surface", () => {
  const headerClassName = threadRailHeaderClassName();
  assert.match(headerClassName, /min-h-13/);
  assert.doesNotMatch(headerClassName, /border-b/);

  const idleEntryClassName = threadRailEntryClassName(false);
  const activeEntryClassName = threadRailEntryClassName(true);
  assert.match(idleEntryClassName, /rounded-lg/);
  assert.match(idleEntryClassName, /hover:bg-muted/);
  assert.match(activeEntryClassName, /bg-muted/);
});

test("projectThreadRailLayout keeps pin count and exposes collapse state", () => {
  const pins = [
    { channelId: "channel-a", rootId: "root-a", pinnedAt: 100 },
    { channelId: "channel-b", rootId: "root-b", pinnedAt: 200 },
  ];

  assert.deepEqual(projectThreadRailLayout({ collapsed: false, pins }, 800), {
    visible: true,
    pinCount: 2,
    collapsed: false,
    collapseControl: { expanded: true, label: "Collapse pinned threads" },
  });
  assert.deepEqual(projectThreadRailLayout({ collapsed: true, pins }, 800), {
    visible: true,
    pinCount: 2,
    collapsed: true,
    collapseControl: { expanded: false, label: "Expand 2 pinned threads" },
  });
});
