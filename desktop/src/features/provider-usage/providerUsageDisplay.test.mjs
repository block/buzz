import assert from "node:assert/strict";
import test from "node:test";

import {
  formatTokenCount,
  providerUsageErrorMessage,
  providerUsageTone,
} from "./providerUsageDisplay.mjs";

test("providerUsageTone follows the remaining-usage thresholds", () => {
  assert.equal(providerUsageTone(62), "healthy");
  assert.equal(providerUsageTone(50), "warning");
  assert.equal(providerUsageTone(20), "warning");
  assert.equal(providerUsageTone(19), "critical");
});

test("formatTokenCount stays compact and handles missing values", () => {
  assert.equal(formatTokenCount(null), "—");
  assert.match(formatTokenCount(13_597_623_776), /13[.,]?6B/i);
});

test("providerUsageErrorMessage never exposes raw app-server details", () => {
  assert.equal(
    providerUsageErrorMessage(
      "codex_not_authenticated: alice@example.com should not render",
    ),
    "Sign in with Codex to show usage",
  );
  assert.equal(
    providerUsageErrorMessage("unknown failure with local path /Users/alice"),
    "Usage temporarily unavailable",
  );
});
