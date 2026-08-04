import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { nostrBindBrowserFinishState } from "./nostrBindFinishState.ts";

describe("nostrBindBrowserFinishState", () => {
  it("names Run402 as the only authoritative completion screen", () => {
    assert.deepEqual(nostrBindBrowserFinishState(null), {
      title: "Check Run402 in your browser",
      description:
        "The Run402 tab showing the six-digit code is the only place that can confirm whether co-ownership completed.",
      fallbackLabel: "Run402 didn’t receive the approval?",
      closeLabel: "Close",
    });
  });

  it("does not claim the browser received anything when opening it failed", () => {
    assert.deepEqual(nostrBindBrowserFinishState("browser unavailable"), {
      title: "The browser did not open",
      description:
        "Buzz signed the approval, but could not open Run402. Use the manual recovery below; no ownership changed.",
      fallbackLabel: "Use manual recovery",
      closeLabel: "Close",
    });
  });
});
