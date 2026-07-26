import assert from "node:assert/strict";
import test from "node:test";

import {
  attestationDescription,
  attestationTitle,
  shortenPubkey,
} from "./SpawnerAttestationDialog.tsx";

const SPAWNER =
  "1111111111111111111111111111111111111111111111111111111111111111";
const AGENT =
  "2222222222222222222222222222222222222222222222222222222222222222";

const request = {
  spawnerPubkey: SPAWNER,
  specSlug: "fizz-prod",
  agentPubkey: AGENT,
  conditions: "",
  nonce: "ab".repeat(32),
  encryptedContent: "ciphertext",
  eventId: "e".repeat(64),
};

test("shortensLongPubkeysButLeavesShortStringsAlone", () => {
  const short = shortenPubkey(SPAWNER);
  assert.ok(short.length < SPAWNER.length);
  assert.ok(short.startsWith("1111111111"));
  assert.equal(shortenPubkey("short"), "short");
});

test("namesBothTheSpawnerAndTheAgentBeingAuthorized", () => {
  // The user has to be able to tell *which* key they are vouching for. Naming
  // only the spawner would let one request authorize any agent.
  const copy = attestationDescription(request);

  assert.ok(copy.includes(shortenPubkey(SPAWNER)));
  assert.ok(copy.includes(shortenPubkey(AGENT)));
  assert.ok(copy.includes("fizz-prod"));
});

test("statesThatApprovingGrantsChannelAccess", () => {
  // Signing an attestation admits the agent under the user's own relay
  // membership. Consent copy that omits this is not informed consent.
  const copy = attestationDescription(request);

  assert.ok(/read your channels/i.test(copy));
  assert.ok(/only approve this if you run this server/i.test(copy));
});

test("hasFallbackCopyWhenNoRequestIsPending", () => {
  // Rendered during the dialog's close animation, after the queue has drained.
  assert.equal(
    attestationDescription(null),
    "A server wants to run an agent for you.",
  );
});

test("saysTheAgentIsMovingAndWillStopLocallyWhenRelocating", () => {
  // The user already owns this key. Telling them a "new agent key" was created
  // would hide the only consequence that matters: the agent leaves this Mac.
  const copy = attestationDescription(request, { isRelocation: true });

  assert.ok(/already run on this Mac/i.test(copy));
  assert.ok(/stop running on this Mac/i.test(copy));
  assert.ok(!/created a new agent key/i.test(copy));
  assert.ok(copy.includes(shortenPubkey(AGENT)));
});

test("saysTheIdentityIsPreservedAcrossTheMove", () => {
  // Losing the key would orphan the agent's memory, so "same identity" is the
  // reassurance that makes approving reasonable.
  const copy = attestationDescription(request, { isRelocation: true });

  assert.ok(/same identity/i.test(copy));
  assert.ok(/memory/i.test(copy));
});

test("keepsTheNewKeyCopyWhenTheAgentIsNotOneWeAlreadyRun", () => {
  const copy = attestationDescription(request, { isRelocation: false });

  assert.ok(/created a new agent key/i.test(copy));
  assert.ok(!/stop running on this Mac/i.test(copy));
});

test("titleDistinguishesAMoveFromANewAuthorization", () => {
  assert.equal(
    attestationTitle({ isRelocation: true }),
    "Move this agent to a server?",
  );
  assert.equal(attestationTitle(), "Authorize a server agent?");
});
