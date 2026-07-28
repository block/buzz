import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  RAFT_ACCENT_HEX,
  RAFT_LIGHT_BACKGROUND,
  RAFT_LIGHT_PRIMARY,
  RAFT_LIGHT_SIDEBAR,
  getRaftShellVars,
} from "./raft-shell.ts";

describe("raft shell tokens", () => {
  it("exports amber accent for Buzz runtime primary", () => {
    assert.equal(RAFT_ACCENT_HEX, "#fbbf24");
  });

  it("light shell matches cream paper / charcoal side / amber primary", () => {
    const light = getRaftShellVars(false);
    assert.equal(light["--background"], RAFT_LIGHT_BACKGROUND);
    assert.equal(light["--primary"], RAFT_LIGHT_PRIMARY);
    assert.equal(light["--sidebar-background"], RAFT_LIGHT_SIDEBAR);
    assert.equal(light["--radius"], "0rem");
    // Cream-ish warm background (hue ~30, high lightness)
    assert.match(light["--background"], /^30 /);
    // Charcoal sidebar (low lightness)
    assert.match(light["--sidebar-background"], /^24 /);
    // Amber primary
    assert.equal(light["--primary"], "43 96% 56%");
  });

  it("dark shell keeps amber primary with warm stone surfaces", () => {
    const dark = getRaftShellVars(true);
    assert.equal(dark["--primary"], "43 96% 56%");
    assert.equal(dark["--background"], "24 10% 10%");
    assert.equal(dark["--sidebar-background"], "24 10% 12%");
  });

  it("returns independent copies (no shared mutation)", () => {
    const a = getRaftShellVars(false);
    const b = getRaftShellVars(false);
    a["--primary"] = "0 0% 0%";
    assert.equal(b["--primary"], "43 96% 56%");
  });
});
