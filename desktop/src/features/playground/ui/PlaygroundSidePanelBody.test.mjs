import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import {
  PLAYGROUND_SIDE_PANEL_ROOT_CLASS,
  playgroundSidePanelChromeIsCapped,
  playgroundSidePanelRootRejectsFullCanvasCover,
} from "../lib/overlayLayout.ts";

const dir = dirname(fileURLToPath(import.meta.url));

test("PlaygroundSidePanelBody root uses capped side-panel class (not inset-0)", () => {
  const source = readFileSync(join(dir, "PlaygroundSidePanelBody.tsx"), "utf8");
  assert.match(source, /PLAYGROUND_SIDE_PANEL_ROOT_CLASS/);
  assert.match(source, /data-playground-host="side-panel"/);
  assert.match(source, /data-testid="playground-side-panel"/);
  assert.doesNotMatch(source, /absolute inset-0/);
  assert.equal(
    playgroundSidePanelRootRejectsFullCanvasCover(PLAYGROUND_SIDE_PANEL_ROOT_CLASS),
    true,
  );
});

test("channel side-panel chrome forces expanded/coverAppChrome false", () => {
  const source = readFileSync(
    join(dir, "useChannelPlaygroundSidePanel.tsx"),
    "utf8",
  );
  assert.match(source, /idleAuxiliaryExpanded:\s*false/);
  assert.match(source, /idleAuxiliaryCoverAppChrome:\s*false/);
  assert.equal(
    playgroundSidePanelChromeIsCapped({
      expanded: false,
      coverAppChrome: false,
      showFullscreen: false,
    }),
    true,
  );
});

test("off-channel side-panel drawer never coverAppChrome / leftPx=0", () => {
  const source = readFileSync(join(dir, "PlaygroundHost.tsx"), "utf8");
  assert.match(source, /coverAppChrome=\{false\}/);
  assert.match(source, /leftPx=\{THREAD_FOCUS_SLIVER_WIDTH_PX\}/);
  assert.doesNotMatch(source, /leftPx=\{0\}/);
  assert.doesNotMatch(source, /coverAppChrome=\{true\}/);
});
