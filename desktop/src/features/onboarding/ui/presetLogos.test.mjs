/**
 * Preset-logo coverage guard.
 *
 * Every tier-2 preset the backend emits must have a bundled logo, or it renders
 * as the generic TerminalSquare fallback next to siblings that show real marks.
 * The two sides live in different languages — Rust `PRESET_HARNESSES` vs the TS
 * `PRESET_LOGOS` record — so no compiler catches drift, and `RuntimeIcon`'s
 * `onError` fallback hides a missing file at runtime. This test reads the Rust
 * source as text (the same trick `motion.test.mjs` uses for CSS) and asserts
 * both directions plus on-disk existence of every mapped file.
 */

import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { PRESET_LOGOS } from "./RuntimeIcon.tsx";

const desktopRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../..",
);

const discoveryRs = readFileSync(
  path.join(desktopRoot, "src-tauri/src/managed_agents/discovery.rs"),
  "utf8",
);

const presetBlock = discoveryRs.match(
  /const PRESET_HARNESSES: &\[PresetHarness\] = &\[([\s\S]*?)\n\];/,
);
assert.ok(presetBlock, "could not locate PRESET_HARNESSES in discovery.rs");

const presetIds = [...presetBlock[1].matchAll(/^\s{8}id: "([^"]+)",$/gm)].map(
  (match) => match[1],
);

test("PRESET_HARNESSES parse found the preset ids", () => {
  // Guards the regex itself: a struct-field rename would otherwise silently
  // yield zero ids and make every assertion below vacuously pass.
  assert.ok(
    presetIds.length >= 8,
    `expected at least 8 preset ids, parsed ${presetIds.length}`,
  );
});

const FALLBACK_ONLY_PRESETS = new Set([
  // Cursor publishes official assets, but neither its brand page nor terms grant
  // third parties permission to redistribute them. Keep the generic icon.
  "cursor",
]);

for (const id of presetIds) {
  test(`preset "${id}" has a bundled logo or an approved fallback`, () => {
    const logoPath = PRESET_LOGOS[id];
    if (FALLBACK_ONLY_PRESETS.has(id)) {
      assert.equal(
        logoPath,
        undefined,
        `preset "${id}" must keep the generic TerminalSquare fallback until ` +
          "its vendor grants logo redistribution permission",
      );
      return;
    }
    assert.ok(
      logoPath,
      `preset "${id}" has no PRESET_LOGOS entry — it renders the generic ` +
        `TerminalSquare fallback. Add desktop/public${logoPath ?? `/harness-logos/${id}.png`} ` +
        `and map it in RuntimeIcon.tsx.`,
    );
    assert.ok(
      existsSync(path.join(desktopRoot, "public", logoPath)),
      `PRESET_LOGOS["${id}"] points at ${logoPath}, which is missing from ` +
        `desktop/public — RuntimeIcon's onError would silently fall back.`,
    );
  });
}

test("PRESET_LOGOS has no entries for unknown presets", () => {
  const unknown = Object.keys(PRESET_LOGOS).filter(
    (id) => !presetIds.includes(id),
  );
  assert.deepEqual(
    unknown,
    [],
    `PRESET_LOGOS maps ids the backend does not emit as presets: ${unknown.join(", ")}`,
  );
});
