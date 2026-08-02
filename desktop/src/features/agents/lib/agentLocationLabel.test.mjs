import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { agentLocationLabel, agentRunsOnLabel } from "./agentLocationLabel.ts";

describe("agentRunsOnLabel", () => {
  it("names the provider for a provider-backed agent", () => {
    assert.equal(
      agentRunsOnLabel({ type: "provider", id: "ssh", config: {} }),
      "ssh",
    );
  });

  it("is silent for a local agent — 'on this computer' is the assumption", () => {
    assert.equal(agentRunsOnLabel({ type: "local" }), null);
  });

  it("is silent when there is no backend yet", () => {
    assert.equal(agentRunsOnLabel(undefined), null);
    assert.equal(agentRunsOnLabel(null), null);
  });

  it("never reads a host out of the provider config", () => {
    // `exclusiveRemoteHarness` refuses to bless an `ssh_host` key; this label
    // must not be the surface that reintroduces it.
    const label = agentRunsOnLabel({
      type: "provider",
      id: "ssh",
      config: { ssh_host: "vps.example.com", ssh_user: "buzz" },
    });
    assert.equal(label, "ssh");
    assert.ok(!label.includes("vps.example.com"));
  });

  it("falls back to a floor rather than rendering an empty label", () => {
    assert.equal(
      agentRunsOnLabel({ type: "provider", id: "  ", config: {} }),
      "Unknown provider",
    );
  });
});

describe("agentLocationLabel", () => {
  it("reads as a compact metadata line", () => {
    assert.equal(
      agentLocationLabel({ type: "provider", id: "ssh", config: {} }),
      "on ssh",
    );
  });

  it("adds nothing for a local agent", () => {
    assert.equal(agentLocationLabel({ type: "local" }), null);
    assert.equal(agentLocationLabel(undefined), null);
  });
});
