import assert from "node:assert/strict";
import test from "node:test";

import {
  claimCoverDrawerFocus,
  hasCoverDrawerFocusClaim,
  releaseCoverDrawerFocus,
} from "./coverDrawerFocusSlot.ts";

test("a fresh claim holds the slot", () => {
  const claim = claimCoverDrawerFocus();

  assert.equal(hasCoverDrawerFocusClaim(claim), true);
});

test("a successor's claim supersedes the outgoing drawer's", () => {
  // The replacement case: the outgoing drawer's restore is deferred a frame,
  // and by the time it runs the incoming drawer has claimed and taken focus.
  const outgoing = claimCoverDrawerFocus();
  const incoming = claimCoverDrawerFocus();

  assert.equal(hasCoverDrawerFocusClaim(outgoing), false);
  assert.equal(hasCoverDrawerFocusClaim(incoming), true);
});

test("only the newest claim holds the slot across a chain of replacements", () => {
  const claims = [
    claimCoverDrawerFocus(),
    claimCoverDrawerFocus(),
    claimCoverDrawerFocus(),
  ];

  const newest = claims.at(-1);
  for (const claim of claims.slice(0, -1)) {
    assert.equal(hasCoverDrawerFocusClaim(claim), false);
  }
  assert.equal(hasCoverDrawerFocusClaim(newest), true);
});

test("releasing invalidates the outstanding claim without granting a new one", () => {
  // The view-mode switch case: nothing replaces the drawer, but the caller has
  // already placed focus, so the drawer's own restore must not fire.
  const claim = claimCoverDrawerFocus();
  releaseCoverDrawerFocus();

  assert.equal(hasCoverDrawerFocusClaim(claim), false);
});

test("a claim taken after a release holds the slot again", () => {
  releaseCoverDrawerFocus();
  const claim = claimCoverDrawerFocus();

  assert.equal(hasCoverDrawerFocusClaim(claim), true);
});

test("claims are never reused, so a stale claim cannot alias a live one", () => {
  const first = claimCoverDrawerFocus();
  releaseCoverDrawerFocus();
  claimCoverDrawerFocus();
  releaseCoverDrawerFocus();
  const later = claimCoverDrawerFocus();

  assert.notEqual(first, later);
  assert.equal(hasCoverDrawerFocusClaim(first), false);
});
