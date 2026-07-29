import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  DEFAULT_SHELL_STYLE,
  SHELL_STYLES,
  SHELL_STYLE_STORAGE_KEY,
  getShellStyle,
  getShellStyleVars,
  isShellStyleId,
} from "./shell-styles.ts";
import {
  RAFT_LIGHT_PRIMARY,
  RAFT_LIGHT_SIDEBAR,
  getRaftShellVars,
} from "./raft-shell.ts";

describe("shell styles catalog", () => {
  it("includes at least 6 styles with Persona5 and Max", () => {
    assert.ok(SHELL_STYLES.length >= 6);
    const ids = new Set(SHELL_STYLES.map((s) => s.id));
    assert.ok(ids.has("raft"));
    assert.ok(ids.has("persona5"));
    assert.ok(ids.has("persona5max"));
  });

  it("defaults to raft and validates ids", () => {
    assert.equal(DEFAULT_SHELL_STYLE, "raft");
    assert.equal(SHELL_STYLE_STORAGE_KEY, "buzz-shell-style");
    assert.equal(isShellStyleId("raft"), true);
    assert.equal(isShellStyleId("persona5max"), true);
    assert.equal(isShellStyleId("nope"), false);
  });

  it("raft shell matches raft-shell source of truth", () => {
    const light = getShellStyleVars("raft", false);
    const dark = getShellStyleVars("raft", true);
    assert.deepEqual(light, getRaftShellVars(false));
    assert.deepEqual(dark, getRaftShellVars(true));
    assert.equal(light["--primary"], RAFT_LIGHT_PRIMARY);
    assert.equal(light["--sidebar-background"], RAFT_LIGHT_SIDEBAR);
  });

  it("persona5 and max force dark chrome with red primary", () => {
    const p5 = getShellStyle("persona5");
    const max = getShellStyle("persona5max");
    assert.equal(p5.forceDark, true);
    assert.equal(max.forceDark, true);
    const p5Vars = getShellStyleVars("persona5", true);
    const maxVars = getShellStyleVars("persona5max", true);
    assert.match(p5Vars["--primary"], /^355 /);
    assert.match(maxVars["--primary"], /^348 /);
    // Near-black sidebar, high-contrast text
    assert.match(p5Vars["--sidebar-background"], /^0 0% [0-4]%/);
    assert.match(p5Vars["--sidebar-foreground"], /^0 0% 9/);
  });

  it("each style returns independent copies", () => {
    for (const style of SHELL_STYLES) {
      const a = getShellStyleVars(style.id, false);
      const b = getShellStyleVars(style.id, false);
      a["--primary"] = "mutated";
      assert.notEqual(b["--primary"], "mutated");
      assert.ok(a["--sidebar-background"]);
      assert.ok(a["--primary"]);
      assert.ok(style.swatch.startsWith("#"));
      assert.ok(style.accentHex.startsWith("#"));
    }
  });
});
