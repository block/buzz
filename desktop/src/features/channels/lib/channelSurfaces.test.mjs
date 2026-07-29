import assert from "node:assert/strict";
import test from "node:test";

import { resolveChannelSurface } from "./channelSurfaces.ts";

const renderPane = () => "pane";

test("exposes chrome even when no surface is active", () => {
  // Regression: gating the whole presentation on `active` hid the tab strip
  // that activates a surface, plus the connect and post-approval dialogs.
  const surface = resolveChannelSurface({
    active: false,
    navigation: "tabs",
    renderPane,
  });
  assert.notEqual(surface, null);
  assert.equal(surface.navigation, "tabs");
  assert.equal(surface.renderContent, undefined);
});

test("takes over content only when active", () => {
  const surface = resolveChannelSurface({
    active: true,
    navigation: "tabs",
    renderPane,
  });
  assert.equal(surface.renderContent, renderPane);
});

test("contributes nothing when inactive with no chrome", () => {
  assert.equal(
    resolveChannelSurface({ active: false, navigation: undefined, renderPane }),
    null,
  );
});
