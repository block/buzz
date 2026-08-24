import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { agentActivityWindowTitle } from "./agentActivityWindow.ts";

describe("agentActivityWindowTitle", () => {
  it("names both the agent and channel", () => {
    assert.equal(
      agentActivityWindowTitle("Mongo", "buzz-inline-chip-wrap"),
      "Mongo · #buzz-inline-chip-wrap",
    );
  });

  it("does not duplicate a supplied channel hash", () => {
    assert.equal(
      agentActivityWindowTitle(" Mongo ", " #design "),
      "Mongo · #design",
    );
  });
});
