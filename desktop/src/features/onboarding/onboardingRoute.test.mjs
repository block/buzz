import assert from "node:assert/strict";
import test from "node:test";

import {
  createOnboardingHistoryState,
  isCommunitySetupRouteStep,
  isMachineOnboardingRouteStep,
  ONBOARDING_ROUTE_STEPS,
  onboardingRoutePath,
  parseOnboardingRouteStep,
  readOnboardingHistoryEntry,
} from "./onboardingRoute.ts";

test("all onboarding route steps round-trip through their paths", () => {
  for (const step of ONBOARDING_ROUTE_STEPS) {
    assert.equal(parseOnboardingRouteStep(onboardingRoutePath(step)), step);
  }
});

test("unknown and nested onboarding paths are rejected", () => {
  assert.equal(parseOnboardingRouteStep("/onboarding/not-a-step"), null);
  assert.equal(parseOnboardingRouteStep("/onboarding/backup/password"), null);
  assert.equal(parseOnboardingRouteStep("/channels/general"), null);
});

test("machine and community route groups do not overlap", () => {
  const machineSteps = ONBOARDING_ROUTE_STEPS.filter((step) =>
    isMachineOnboardingRouteStep(step),
  );
  const communitySteps = ONBOARDING_ROUTE_STEPS.filter((step) =>
    isCommunitySetupRouteStep(step),
  );

  assert.deepEqual(machineSteps, [
    "identity",
    "key-import",
    "backup",
    "backup-options",
    "backup-password",
    "agents",
    "defaults",
  ]);
  assert.deepEqual(communitySteps, [
    "community",
    "community-existing",
    "community-join",
    "community-member",
    "community-hosted",
    "community-owned",
  ]);
  assert.equal(
    machineSteps.some((step) => communitySteps.includes(step)),
    false,
  );
});

test("history entries are accepted only for the active session", () => {
  const state = createOnboardingHistoryState("session-a", 3);

  assert.deepEqual(
    readOnboardingHistoryEntry(
      "/onboarding/backup-options",
      state,
      "session-a",
    ),
    { depth: 3, step: "backup-options" },
  );
  assert.equal(
    readOnboardingHistoryEntry(
      "/onboarding/backup-options",
      state,
      "session-b",
    ),
    null,
  );
});

test("direct, malformed, and stale entries require canonical repair", () => {
  assert.equal(
    readOnboardingHistoryEntry("/onboarding/identity", {}, "session-a"),
    null,
  );
  assert.equal(
    readOnboardingHistoryEntry(
      "/onboarding/identity",
      createOnboardingHistoryState("session-a", -1),
      "session-a",
    ),
    null,
  );
  assert.equal(
    readOnboardingHistoryEntry(
      "/onboarding/unknown",
      createOnboardingHistoryState("session-a", 0),
      "session-a",
    ),
    null,
  );
});

test("restored entry depths preserve the active branch position", () => {
  const earlier = createOnboardingHistoryState("session-a", 2);
  const later = createOnboardingHistoryState("session-a", 3);

  assert.equal(
    readOnboardingHistoryEntry("/onboarding/agents", earlier, "session-a")
      ?.depth,
    2,
  );
  assert.equal(
    readOnboardingHistoryEntry("/onboarding/defaults", later, "session-a")
      ?.depth,
    3,
  );
});
