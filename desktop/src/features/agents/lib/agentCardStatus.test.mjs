import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  deriveAgentCardStatus,
  formatAgentCardActivityChannel,
} from "./agentCardStatus.ts";

describe("deriveAgentCardStatus", () => {
  it("shows working only for an active runtime with real working activity", () => {
    assert.equal(
      deriveAgentCardStatus({
        hasError: false,
        isWorking: true,
        status: "running",
      }),
      "working",
    );
  });

  it("shows available for an active idle runtime", () => {
    assert.equal(
      deriveAgentCardStatus({
        hasError: false,
        isWorking: false,
        status: "deployed",
      }),
      "available",
    );
  });

  it("prioritizes an inactive runtime error over stale working activity", () => {
    assert.equal(
      deriveAgentCardStatus({
        hasError: true,
        isWorking: true,
        status: "stopped",
      }),
      "error",
    );
  });

  it("shows off for an inactive runtime without an error", () => {
    assert.equal(
      deriveAgentCardStatus({
        hasError: false,
        isWorking: true,
        status: "not_deployed",
      }),
      "off",
    );
  });

  it("shows off for a persona without a spawned runtime", () => {
    assert.equal(
      deriveAgentCardStatus({
        hasError: false,
        isWorking: false,
        status: null,
      }),
      "off",
    );
  });
});

describe("formatAgentCardActivityChannel", () => {
  it("shows a channel name only when the viewer has a resolved visible name", () => {
    assert.equal(formatAgentCardActivityChannel("general"), "#general");
    assert.equal(formatAgentCardActivityChannel(undefined), "activiteit");
    assert.equal(formatAgentCardActivityChannel(""), "activiteit");
  });
});
