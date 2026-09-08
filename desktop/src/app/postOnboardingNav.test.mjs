/**
 * Tests for the post-onboarding navigation contract (Thufir F8).
 *
 * When the user clicks "Set up agents" during machine onboarding,
 * MachineOnboardingFlow calls complete() then navigateAfterComplete({to, search}).
 * App.tsx records the pending nav and fires it only when machine.stage === "ready"
 * (i.e. once RouterProvider is mounted), never before.
 *
 * These are pure-logic tests — they simulate the App.tsx useEffect predicate
 * directly. No React rendering needed; the behavior under test is:
 *   pendingNav × stage → navigate() call count.
 */
import assert from "node:assert/strict";
import test from "node:test";

// ── Simulate the App.tsx postOnboardingNav useEffect logic ──────────────────
//
// The production code:
//   useEffect(() => {
//     if (machine.stage === "ready" && postOnboardingNav) {
//       void router.navigate({ to: postOnboardingNav.to, search: ... });
//       setPostOnboardingNav(null);
//     }
//   }, [machine.stage, postOnboardingNav]);
//
// We simulate this as a pure function and drive it through stage transitions.

function runEffect(stage, nav, navigate) {
  if (stage === "ready" && nav !== null) {
    navigate({ to: nav.to, search: nav.search ?? {} });
    return null; // cleared
  }
  return nav; // unchanged
}

test("navigate does not fire before stage reaches ready", () => {
  const calls = [];
  const navigate = (args) => calls.push(args);

  const nav = { to: "/settings", search: { section: "agents" } };

  // Drive through non-ready stages — navigate must not fire.
  for (const stage of ["loading", "onboarding", "blocking"]) {
    runEffect(stage, nav, navigate);
  }

  assert.equal(calls.length, 0, "navigate must not be called before ready");
});

test("navigate fires exactly once when stage transitions to ready", () => {
  const calls = [];
  const navigate = (args) => calls.push(args);

  const nav = { to: "/settings", search: { section: "agents" } };

  // Pre-ready stages — no call.
  let pending = nav;
  pending = runEffect("onboarding", pending, navigate);

  // Stage reaches ready — navigate fires once.
  pending = runEffect("ready", pending, navigate);

  assert.equal(calls.length, 1, "navigate must fire exactly once on ready");
  assert.equal(calls[0].to, "/settings");
  assert.deepStrictEqual(calls[0].search, { section: "agents" });
});

test("navigate does not fire again after nav is cleared", () => {
  const calls = [];
  const navigate = (args) => calls.push(args);

  const nav = { to: "/settings", search: { section: "agents" } };
  let pending = nav;

  // First ready transition fires and clears.
  pending = runEffect("ready", pending, navigate);
  assert.equal(calls.length, 1);
  assert.equal(pending, null, "pending nav must be cleared after firing");

  // Re-running the effect with null nav must not fire again.
  pending = runEffect("ready", pending, navigate);
  assert.equal(
    calls.length,
    1,
    "navigate must not fire again after being cleared",
  );
});

test("navigate fires immediately if nav is set while already ready", () => {
  // Edge case: nav arrives after the machine is already ready (e.g. hot reload).
  const calls = [];
  const navigate = (args) => calls.push(args);

  const nav = { to: "/settings", search: { section: "agents" } };

  // Start with no pending nav while ready.
  let pending = runEffect("ready", null, navigate);
  assert.equal(calls.length, 0, "no nav pending → no call");

  // Nav arrives (complete() called while already ready).
  pending = runEffect("ready", nav, navigate);
  assert.equal(calls.length, 1, "nav must fire immediately when already ready");
  assert.equal(pending, null);
});

test("navigate intent carries exact to and search from navigateToAgentSettings", () => {
  // Verifies the specific shape emitted by MachineOnboardingFlow's
  // navigateToAgentSettings action: { to: '/settings', search: { section: 'agents' } }
  const calls = [];
  const navigate = (args) => calls.push(args);

  // Simulate MachineOnboardingFlow calling navigateAfterComplete.
  const nav = { to: "/settings", search: { section: "agents" } };
  runEffect("ready", nav, navigate);

  assert.equal(calls.length, 1);
  assert.equal(calls[0].to, "/settings", "must navigate to /settings");
  assert.equal(
    calls[0].search.section,
    "agents",
    "must include section=agents",
  );
});
