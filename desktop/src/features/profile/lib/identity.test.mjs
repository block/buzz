import assert from "node:assert/strict";
import test from "node:test";

import { profileLookupsEqual } from "./identity.ts";

const summary = (over = {}) => ({
  displayName: "Ada",
  avatarUrl: "https://x/a.png",
  nip05Handle: "ada@x",
  ownerPubkey: null,
  isAgent: false,
  ...over,
});

test("profileLookupsEqual: same reference is equal", () => {
  const a = { p1: summary() };
  assert.equal(profileLookupsEqual(a, a), true);
});

test("profileLookupsEqual: distinct objects, identical values are equal", () => {
  assert.equal(profileLookupsEqual({ p1: summary() }, { p1: summary() }), true);
});

test("profileLookupsEqual: different key count is not equal", () => {
  assert.equal(
    profileLookupsEqual({ p1: summary() }, { p1: summary(), p2: summary() }),
    false,
  );
});

test("profileLookupsEqual: same count, different keys is not equal", () => {
  assert.equal(
    profileLookupsEqual({ p1: summary() }, { p2: summary() }),
    false,
  );
});

test("profileLookupsEqual: a changed field is not equal", () => {
  for (const field of [
    "displayName",
    "avatarUrl",
    "nip05Handle",
    "ownerPubkey",
    "isAgent",
  ]) {
    assert.equal(
      profileLookupsEqual(
        { p1: summary() },
        { p1: summary({ [field]: field === "isAgent" ? true : "changed" }) },
      ),
      false,
      `field ${field} should break equality`,
    );
  }
});

test("profileLookupsEqual: two empty lookups are equal", () => {
  assert.equal(profileLookupsEqual({}, {}), true);
});

// Render-count proof for the Tier-1 typing-storm fix (#1533 discipline).
// MessageRow re-renders iff `prev.profiles === next.profiles` fails, so the
// stabiliser's job is: hold the reference across value-equal re-derives (the
// per-keystroke churn) and release it only on a real value change. This
// replays the exact ChannelScreen ref idiom against a sequence of freshly
// built lookups and asserts reference identity == render decision.
function makeStabiliser() {
  let ref;
  let first = true;
  return (raw) => {
    if (first || !profileLookupsEqual(ref, raw)) {
      ref = raw;
    }
    first = false;
    return ref;
  };
}

test("stabiliser: value-equal re-derives keep the same reference (no re-render)", () => {
  const stabilise = makeStabiliser();
  // Each entry is a fresh object identity — exactly what a users-batch re-key
  // produces on every keystroke-adjacent typing event.
  const first = stabilise({ p1: summary() });
  const churnA = stabilise({ p1: summary() });
  const churnB = stabilise({ p1: summary() });
  assert.equal(churnA, first, "value-equal churn must not swap the reference");
  assert.equal(churnB, first, "repeated churn must not swap the reference");
});

test("stabiliser: a real profile change swaps the reference (re-render fires)", () => {
  const stabilise = makeStabiliser();
  const first = stabilise({ p1: summary() });
  const changed = stabilise({ p1: summary({ displayName: "Grace" }) });
  assert.notEqual(
    changed,
    first,
    "a real value change must swap the reference",
  );
  // ...and then re-stabilises around the new value.
  const held = stabilise({ p1: summary({ displayName: "Grace" }) });
  assert.equal(held, changed, "must re-stabilise around the new value");
});
