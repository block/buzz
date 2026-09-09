import assert from "node:assert/strict";
import test from "node:test";

const { pluginBrowserSurfaceKey } = await import(
  "./plugins.$pluginId.$contributionId.tsx"
);

test("the key varies with pluginId alone", () => {
  assert.notEqual(
    pluginBrowserSurfaceKey("plugin-a", "contrib-1"),
    pluginBrowserSurfaceKey("plugin-b", "contrib-1"),
  );
});

test("the key varies with contributionId alone", () => {
  // A key derived from only pluginId would fail to remount BrowserSurface
  // when navigating between two contributions of the *same* plugin.
  assert.notEqual(
    pluginBrowserSurfaceKey("plugin-a", "contrib-1"),
    pluginBrowserSurfaceKey("plugin-a", "contrib-2"),
  );
});

test("the same pair produces the same key — no spurious remount", () => {
  assert.equal(
    pluginBrowserSurfaceKey("plugin-a", "contrib-1"),
    pluginBrowserSurfaceKey("plugin-a", "contrib-1"),
  );
});
