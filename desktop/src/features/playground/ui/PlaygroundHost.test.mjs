import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

/**
 * Off-channel Browsers Open must use the same FocusThreadDrawer slide-out as
 * channel pin Open / card Open — not a narrow docked RHS AuxiliaryPanel column.
 */
test("off-channel side-panel host uses FocusThreadDrawer (not docked RHS column)", () => {
  const dir = dirname(fileURLToPath(import.meta.url));
  const source = readFileSync(join(dir, "PlaygroundHost.tsx"), "utf8");
  const hostIdx = source.indexOf(
    'data-testid="playground-offchannel-side-panel-host"',
  );
  assert.ok(hostIdx > 0, "off-channel host test id present");
  const window = source.slice(Math.max(0, hostIdx - 120), hostIdx + 900);
  assert.match(window, /absolute inset-0/);
  assert.match(window, /FocusThreadDrawer/);
  assert.match(window, /coverAppChrome=\{false\}/);
  assert.match(window, /isFocusDrawer/);
  assert.match(window, /THREAD_FOCUS_SLIVER_WIDTH_PX/);
  assert.doesNotMatch(window, /justify-end/);
  assert.doesNotMatch(window, /AUXILIARY_PANEL_DEFAULT_WIDTH_PX/);
});

test("main-app host never falls through to PlaygroundOverlay (product lock)", () => {
  const dir = dirname(fileURLToPath(import.meta.url));
  const source = readFileSync(join(dir, "PlaygroundHost.tsx"), "utf8");
  // Non-popout path must return FocusThreadDrawer / null — never mount
  // PlaygroundOverlay for overlayHost "window" full-canvas fill.
  const fnStart = source.indexOf("export function PlaygroundHost");
  assert.ok(fnStart >= 0);
  const body = source.slice(fnStart);
  const nonPopoutGuard = body.indexOf("if (!popout)");
  assert.ok(nonPopoutGuard > 0, "non-popout guard present");
  const overlayMount = body.indexOf("<PlaygroundOverlay");
  assert.ok(overlayMount > nonPopoutGuard);
  // PlaygroundOverlay only after the popout-only lockPlacement block.
  assert.match(body.slice(nonPopoutGuard, overlayMount), /OS \/ embedded pop-out only|popout\.kind/);
  assert.doesNotMatch(
    body.slice(nonPopoutGuard, overlayMount),
    /isPlaygroundSidePanelHost\(\)\s*\)\s*\{[^}]*PlaygroundOverlay/s,
  );
});
