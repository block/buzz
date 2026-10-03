import assert from "node:assert/strict";
import test from "node:test";

import {
  SYNTAX_THEMES,
  extractThemeInfo,
  loadThemeData,
} from "./theme-loader.ts";

test("comment color comes from the theme's own scope rules", async () => {
  const info = extractThemeInfo(
    "github-light",
    await loadThemeData("github-light"),
  );

  // GitHub Light styles comments grey and body text near-black. Reading scope
  // rules from the wrong field silently returned the foreground for both.
  assert.equal(info.comment.toLowerCase(), "#6a737d");
  assert.notEqual(info.comment.toLowerCase(), info.fg.toLowerCase());
});

test("bundled themes keep muted text distinct from body text", async () => {
  const collapsed = [];

  for (const name of SYNTAX_THEMES) {
    const info = extractThemeInfo(name, await loadThemeData(name));
    if (info.comment.toLowerCase() === info.fg.toLowerCase()) {
      collapsed.push(name);
    }
  }

  // A few minimal themes genuinely paint comments in the body text color; the
  // regression this guards against collapsed every single theme.
  assert.ok(
    collapsed.length <= 8,
    `muted text collapsed onto body text in ${collapsed.length} themes: ${collapsed.join(", ")}`,
  );
});
