import assert from "node:assert/strict";
import test from "node:test";

import { hasCurrentRelayBindingForAuthor } from "./currentRelayBinding.ts";

const DISPLAYED_ACTOR = "a".repeat(64);
const EVENT_SIGNER = "b".repeat(64);
const projection = { eventAuthorPubkey: EVENT_SIGNER, freshUntil: 200 };

test("badges only the exact raw event signer", () => {
  assert.equal(
    hasCurrentRelayBindingForAuthor(projection, EVENT_SIGNER, 100),
    true,
  );
  assert.equal(
    hasCurrentRelayBindingForAuthor(
      projection,
      EVENT_SIGNER.toUpperCase(),
      100,
    ),
    false,
  );
  assert.equal(
    hasCurrentRelayBindingForAuthor(
      { eventAuthorPubkey: DISPLAYED_ACTOR, freshUntil: 200 },
      EVENT_SIGNER,
      100,
    ),
    false,
  );
  assert.equal(hasCurrentRelayBindingForAuthor(null, EVENT_SIGNER, 100), false);
});

test("render predicate rechecks the exclusive freshness deadline", () => {
  assert.equal(
    hasCurrentRelayBindingForAuthor(projection, EVENT_SIGNER, 199.999),
    true,
  );
  assert.equal(
    hasCurrentRelayBindingForAuthor(projection, EVENT_SIGNER, 200),
    false,
  );
  assert.equal(
    hasCurrentRelayBindingForAuthor(projection, EVENT_SIGNER, Number.NaN),
    false,
  );
});
