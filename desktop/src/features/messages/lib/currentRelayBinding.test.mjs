import assert from "node:assert/strict";
import test from "node:test";

import { hasCurrentRelayBindingForAuthor } from "./currentRelayBinding.ts";

const DISPLAYED_ACTOR_PUBKEY = "a".repeat(64);
const EVENT_SIGNER_PUBKEY = "b".repeat(64);
const OTHER_PUBKEY = "c".repeat(64);

const projection = {
  eventAuthorPubkey: EVENT_SIGNER_PUBKEY,
};

test("returns false without a current projection", () => {
  assert.equal(
    hasCurrentRelayBindingForAuthor(null, EVENT_SIGNER_PUBKEY),
    false,
  );
});

test("returns true for the exact event author", () => {
  assert.equal(
    hasCurrentRelayBindingForAuthor(projection, EVENT_SIGNER_PUBKEY),
    true,
  );
});

test("does not normalize author case", () => {
  assert.equal(
    hasCurrentRelayBindingForAuthor(
      projection,
      EVENT_SIGNER_PUBKEY.toUpperCase(),
    ),
    false,
  );
});

test("does not trim author whitespace", () => {
  assert.equal(
    hasCurrentRelayBindingForAuthor(projection, ` ${EVENT_SIGNER_PUBKEY}`),
    false,
  );
  assert.equal(
    hasCurrentRelayBindingForAuthor(projection, `${EVENT_SIGNER_PUBKEY} `),
    false,
  );
});

test("returns false for a different or absent event author", () => {
  assert.equal(
    hasCurrentRelayBindingForAuthor(projection, OTHER_PUBKEY),
    false,
  );
  assert.equal(hasCurrentRelayBindingForAuthor(projection, null), false);
  assert.equal(hasCurrentRelayBindingForAuthor(projection, undefined), false);
});

test("uses the raw event signer instead of a relay-attributed display actor", () => {
  const relayAttributedMessage = {
    pubkey: DISPLAYED_ACTOR_PUBKEY,
    signerPubkey: EVENT_SIGNER_PUBKEY,
  };
  const displayedActorProjection = {
    eventAuthorPubkey: DISPLAYED_ACTOR_PUBKEY,
  };

  assert.equal(
    relayAttributedMessage.pubkey,
    displayedActorProjection.eventAuthorPubkey,
    "the displayed actor matches the projection in this regression case",
  );
  assert.equal(
    hasCurrentRelayBindingForAuthor(
      displayedActorProjection,
      relayAttributedMessage.signerPubkey,
    ),
    false,
    "a displayed-actor match must not badge an event signed by another key",
  );
  assert.equal(
    hasCurrentRelayBindingForAuthor(
      projection,
      relayAttributedMessage.signerPubkey,
    ),
    true,
    "an exact raw event-signer match may display the badge",
  );
});
