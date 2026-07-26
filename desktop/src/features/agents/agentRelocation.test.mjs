import assert from "node:assert/strict";
import test from "node:test";

import {
  buildRelocationPlan,
  isRelocationOfLocalAgent,
} from "./agentRelocation.ts";

const AGENT_PUBKEY =
  "3333333333333333333333333333333333333333333333333333333333333333";

test("carriesTheExistingPubkeySoTheSpawnerReusesTheIdentity", () => {
  // Without `agentPubkey` the spawner mints a fresh key, which orphans the
  // agent's NIP-AE memory forever. This field is the entire relocation.
  const { slug, spec } = buildRelocationPlan({
    pubkey: AGENT_PUBKEY,
    name: "Fizz (prod)",
    personaId: "builtin:fizz",
  });

  assert.equal(spec.agentPubkey, AGENT_PUBKEY);
  assert.equal(slug, "fizz-prod");
  assert.equal(spec.name, "Fizz (prod)");
  assert.equal(spec.enabled, true);
});

test("neverPutsASecretOrAPromptOnTheSpec", () => {
  // A kind:30178 spec is world-readable. Anything secret-shaped here is a leak.
  const { spec } = buildRelocationPlan({
    pubkey: AGENT_PUBKEY,
    name: "Honey",
    personaId: "builtin:honey",
  });

  const serialized = JSON.stringify(spec);
  assert.ok(!/nsec|secret|privateKey|seckey/i.test(serialized));
  assert.equal(spec.systemPrompt, undefined);
});

test("publishesThePersonaRelayAddressNotTheRawId", () => {
  // `builtin:fizz` cannot be a `d` tag — the relay rejects the colon — so a raw
  // id would point at a persona that cannot exist.
  const { spec } = buildRelocationPlan({
    pubkey: AGENT_PUBKEY,
    name: "Fizz",
    personaId: "builtin:fizz",
  });

  assert.equal(spec.personaId, "builtin-fizz");
});

test("fallsBackToAPlaceholderPromptWhenTheAgentHasNoPersona", () => {
  // The spec still has to validate; the real prompt arrives over the handshake.
  const { spec } = buildRelocationPlan({
    pubkey: AGENT_PUBKEY,
    name: "Custom",
    personaId: null,
  });

  assert.equal(spec.personaId, undefined);
  assert.equal(typeof spec.systemPrompt, "string");
});

test("refusesANameThatYieldsNoUsableSlug", () => {
  // The slug becomes a container and volume name on the host.
  assert.throws(
    () => buildRelocationPlan({ pubkey: AGENT_PUBKEY, name: "???" }),
    /no usable server name/i,
  );
});

test("detectsARelocationRegardlessOfHexCasing", () => {
  assert.equal(
    isRelocationOfLocalAgent(AGENT_PUBKEY.toUpperCase(), [AGENT_PUBKEY]),
    true,
  );
  assert.equal(isRelocationOfLocalAgent(AGENT_PUBKEY, []), false);
  assert.equal(isRelocationOfLocalAgent(null, [AGENT_PUBKEY]), false);
  assert.equal(isRelocationOfLocalAgent(undefined, [AGENT_PUBKEY]), false);
});
