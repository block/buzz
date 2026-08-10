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

import { RUNTIME_MARKS } from "./HarnessMarks.tsx";
import { PRESET_LOGOS } from "./RuntimeIcon.tsx";

const desktopRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../..",
);

const presetsRs = readFileSync(
  path.join(desktopRoot, "src-tauri/src/managed_agents/discovery/presets.rs"),
  "utf8",
);

const presetBlock = presetsRs.match(
  /const PRESET_HARNESSES: &\[PresetHarness\] = &\[([\s\S]*?)\n\];/,
);
assert.ok(presetBlock, "could not locate PRESET_HARNESSES in presets.rs");

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

for (const id of presetIds) {
  test(`preset "${id}" has a bundled logo or inline mark`, () => {
    // Inline SVG marks (RUNTIME_MARKS) take precedence over bitmap logos —
    // e.g. Cursor's mark ships as an inline CC0 simple-icons path, not a
    // file under desktop/public.
    if (RUNTIME_MARKS[id]) {
      return;
    }
    const logoPath = PRESET_LOGOS[id];
    assert.ok(
      logoPath,
      `preset "${id}" has no RUNTIME_MARKS or PRESET_LOGOS entry — it renders ` +
        `the generic TerminalSquare fallback. Add desktop/public${logoPath ?? `/harness-logos/${id}.png`} ` +
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

test("codex ships no bundled mark or logo (vendor-removed OpenAI blossom)", () => {
  // The OpenAI blossom was removed from simple-icons v16 at the vendor's
  // request — Codex must render RuntimeIcon's neutral terminal-glyph
  // fallback, not a re-bundled copy of the withdrawn mark.
  assert.equal(
    RUNTIME_MARKS.codex,
    undefined,
    "codex has a RUNTIME_MARKS entry — the OpenAI blossom must not ship without explicit approval",
  );
  assert.equal(
    PRESET_LOGOS.codex,
    undefined,
    "codex has a PRESET_LOGOS entry — no bundled Codex logo is approved",
  );
});
