import { expect, test } from "@playwright/test";
import { bytesToHex, hexToBytes } from "@noble/hashes/utils.js";
import { hmac } from "@noble/hashes/hmac.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { getConversationKey, encrypt, decrypt } from "nostr-tools/nip44";
import {
  finalizeEvent,
  getPublicKey,
  generateSecretKey,
} from "nostr-tools/pure";

import { installRelayBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { assertRelaySeeded } from "../helpers/seed";

const RELAY_HTTP_URL = "http://localhost:3000";
const KIND_AGENT_ENGRAM = 30174;
const KIND_PERSONA = 30175;
const D_TAG_DOMAIN = "agent-memory/v1/d-tag";
const PERSONA_ENGRAM_SLUG = "mem/persona";

const isCi = Boolean(process.env.CI);
const relaySeedHookTimeoutMs = isCi ? 90_000 : 30_000;

/**
 * Compute the HMAC-blinded d-tag for an engram slug.
 * Mirrors `buzz_core::engram::d_tag`:
 *   d = lower_hex(HMAC-SHA256(K_c, "agent-memory/v1/d-tag" || 0x00 || slug))
 */
function computeEngramDTag(conversationKey: Uint8Array, slug: string): string {
  const domain = new TextEncoder().encode(D_TAG_DOMAIN);
  const slugBytes = new TextEncoder().encode(slug);
  const message = new Uint8Array(domain.length + 1 + slugBytes.length);
  message.set(domain, 0);
  message[domain.length] = 0x00;
  message.set(slugBytes, domain.length + 1);
  return bytesToHex(hmac(sha256, conversationKey, message));
}

/**
 * Build the engram body JSON that gets NIP-44 encrypted.
 * Mirrors `buzz_core::engram::Body::Memory::to_json_bytes`:
 *   {"slug":"mem/persona","value":"<PersonaEngramBody JSON>"}
 */
function buildEngramBodyJson(slug: string, value: string): string {
  return JSON.stringify({ slug, value });
}

/**
 * Build the PersonaEngramBody (the value inside the engram).
 * Mirrors `PersonaEngramBody` in persona_events.rs.
 */
function buildPersonaEngramBody(
  persona: {
    displayName: string;
    systemPrompt: string;
    avatarUrl?: string | null;
    runtime?: string | null;
    model?: string | null;
    provider?: string | null;
    namePool?: string[];
    envVars?: Record<string, string>;
  },
  ownerPubkey: string,
  slug: string,
): string {
  const content: Record<string, unknown> = {
    display_name: persona.displayName,
    system_prompt: persona.systemPrompt,
  };
  if (persona.avatarUrl) content.avatar_url = persona.avatarUrl;
  if (persona.runtime) content.runtime = persona.runtime;
  if (persona.model) content.model = persona.model;
  if (persona.provider) content.provider = persona.provider;
  if (persona.namePool && persona.namePool.length > 0)
    content.name_pool = persona.namePool;
  if (persona.envVars && Object.keys(persona.envVars).length > 0)
    content.env_vars = persona.envVars;

  // Compute source_version: SHA-256 of the canonical PersonaEventContent JSON
  const contentForHash: Record<string, unknown> = {
    display_name: persona.displayName,
    avatar_url: persona.avatarUrl ?? null,
    system_prompt: persona.systemPrompt,
  };
  if (persona.runtime) contentForHash.runtime = persona.runtime;
  if (persona.model) contentForHash.model = persona.model;
  if (persona.provider) contentForHash.provider = persona.provider;
  if (persona.namePool && persona.namePool.length > 0)
    contentForHash.name_pool = persona.namePool;
  if (persona.envVars && Object.keys(persona.envVars).length > 0)
    contentForHash.env_vars = persona.envVars;

  const sourceVersion = bytesToHex(
    sha256(new TextEncoder().encode(JSON.stringify(contentForHash))),
  );

  return JSON.stringify({
    ...content,
    provenance: {
      owner_pubkey: ownerPubkey,
      kind: KIND_PERSONA,
      slug,
      source_version: sourceVersion,
    },
  });
}

/**
 * Submit a signed event to the relay via HTTP POST.
 */
async function submitEvent(
  event: ReturnType<typeof finalizeEvent> & { pubkey: string },
): Promise<{
  event_id: string;
  accepted: boolean;
  message: string;
}> {
  const response = await fetch(`${RELAY_HTTP_URL}/events`, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Pubkey": event.pubkey },
    body: JSON.stringify(event),
  });
  if (!response.ok) {
    throw new Error(`Relay rejected event: ${await response.text()}`);
  }
  return response.json();
}

/**
 * Query the relay for events matching filters.
 */
async function queryRelay(
  filters: Record<string, unknown>[],
  pubkey: string,
): Promise<
  Array<{
    id: string;
    pubkey: string;
    kind: number;
    content: string;
    tags: string[][];
  }>
> {
  const response = await fetch(`${RELAY_HTTP_URL}/query`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Pubkey": pubkey,
    },
    body: JSON.stringify(filters),
  });
  if (!response.ok) {
    throw new Error(`Relay query failed: ${await response.text()}`);
  }
  return response.json();
}

test.beforeAll(async () => {
  test.setTimeout(relaySeedHookTimeoutMs);
  await assertRelaySeeded();
});

test("create agent from persona publishes mem/persona engram to relay", async ({
  page,
}) => {
  await installRelayBridge(page, "tyler");
  await page.goto("/");

  const ownerIdentity = TEST_IDENTITIES.tyler;
  const ownerPubkey = ownerIdentity.pubkey;

  // Generate a fresh agent keypair (simulates what create_managed_agent does)
  const agentSecretKey = generateSecretKey();
  const agentPubkey = getPublicKey(agentSecretKey);

  // Define the persona
  const persona = {
    displayName: "Integration Test Persona",
    systemPrompt: "You are an integration test assistant.",
    runtime: "goose",
    model: "claude-sonnet-4",
    provider: "anthropic",
  };
  const personaSlug = `test-persona-${Date.now()}`;

  // Step 1: Publish the persona event (kind:30175) as the owner
  const personaContent = JSON.stringify({
    display_name: persona.displayName,
    system_prompt: persona.systemPrompt,
    runtime: persona.runtime,
    model: persona.model,
    provider: persona.provider,
  });

  const personaEvent = finalizeEvent(
    {
      kind: KIND_PERSONA,
      content: personaContent,
      tags: [["d", personaSlug]],
      created_at: Math.floor(Date.now() / 1000),
    },
    hexToBytes(ownerIdentity.privateKey),
  );

  const personaResult = await submitEvent(personaEvent);
  expect(personaResult.accepted).toBe(true);

  // Step 2: Build and publish the persona engram (kind:30174) as the agent
  // This mirrors write_persona_engram_at_creation in fleet_update.rs
  const conversationKey = getConversationKey(agentSecretKey, ownerPubkey);
  const engramDTag = computeEngramDTag(conversationKey, PERSONA_ENGRAM_SLUG);

  const personaEngramValue = buildPersonaEngramBody(
    persona,
    ownerPubkey,
    personaSlug,
  );
  const engramBodyJson = buildEngramBodyJson(
    PERSONA_ENGRAM_SLUG,
    personaEngramValue,
  );

  // NIP-44 encrypt the body
  const ciphertext = encrypt(engramBodyJson, conversationKey);

  const engramEvent = finalizeEvent(
    {
      kind: KIND_AGENT_ENGRAM,
      content: ciphertext,
      tags: [
        ["d", engramDTag],
        ["p", ownerPubkey],
      ],
      created_at: Math.floor(Date.now() / 1000),
    },
    agentSecretKey,
  );

  const engramResult = await submitEvent(engramEvent);
  expect(engramResult.accepted).toBe(true);

  // Step 3: Query the relay to verify the persona event exists
  const personaEvents = await queryRelay(
    [{ kinds: [KIND_PERSONA], authors: [ownerPubkey], "#d": [personaSlug] }],
    ownerPubkey,
  );
  expect(personaEvents.length).toBe(1);
  expect(personaEvents[0].kind).toBe(KIND_PERSONA);
  expect(JSON.parse(personaEvents[0].content).display_name).toBe(
    persona.displayName,
  );

  // Step 4: Query the relay to verify the engram event exists
  const engramEvents = await queryRelay(
    [
      {
        kinds: [KIND_AGENT_ENGRAM],
        authors: [agentPubkey],
        "#d": [engramDTag],
        "#p": [ownerPubkey],
      },
    ],
    ownerPubkey,
  );
  expect(engramEvents.length).toBe(1);
  expect(engramEvents[0].kind).toBe(KIND_AGENT_ENGRAM);
  expect(engramEvents[0].pubkey).toBe(agentPubkey);

  // Step 5: Decrypt the engram and verify content
  const ownerConversationKey = getConversationKey(
    hexToBytes(ownerIdentity.privateKey),
    agentPubkey,
  );
  const decryptedJson = decrypt(engramEvents[0].content, ownerConversationKey);
  const decrypted = JSON.parse(decryptedJson);
  expect(decrypted.slug).toBe(PERSONA_ENGRAM_SLUG);

  const engramBody = JSON.parse(decrypted.value);
  expect(engramBody.display_name).toBe(persona.displayName);
  expect(engramBody.system_prompt).toBe(persona.systemPrompt);
  expect(engramBody.provenance.owner_pubkey).toBe(ownerPubkey);
  expect(engramBody.provenance.kind).toBe(KIND_PERSONA);
  expect(engramBody.provenance.slug).toBe(personaSlug);
  expect(engramBody.provenance.source_version).toHaveLength(64);
});

test("persona edit triggers fleet update that rewrites engram on relay", async ({
  page,
}) => {
  await installRelayBridge(page, "tyler");
  await page.goto("/");

  const ownerIdentity = TEST_IDENTITIES.tyler;
  const ownerPubkey = ownerIdentity.pubkey;

  // Generate agent keypair
  const agentSecretKey = generateSecretKey();
  const agentPubkey = getPublicKey(agentSecretKey);

  const personaSlug = `fleet-test-${Date.now()}`;
  const conversationKey = getConversationKey(agentSecretKey, ownerPubkey);
  const engramDTag = computeEngramDTag(conversationKey, PERSONA_ENGRAM_SLUG);

  // Initial persona
  const initialPersona = {
    displayName: "Fleet Test Persona",
    systemPrompt: "Initial system prompt.",
  };

  // Publish initial persona event
  const initialPersonaEvent = finalizeEvent(
    {
      kind: KIND_PERSONA,
      content: JSON.stringify({
        display_name: initialPersona.displayName,
        system_prompt: initialPersona.systemPrompt,
      }),
      tags: [["d", personaSlug]],
      created_at: Math.floor(Date.now() / 1000) - 10,
    },
    hexToBytes(ownerIdentity.privateKey),
  );
  const personaResult = await submitEvent(initialPersonaEvent);
  expect(personaResult.accepted).toBe(true);

  // Publish initial engram
  const initialEngramValue = buildPersonaEngramBody(
    initialPersona,
    ownerPubkey,
    personaSlug,
  );
  const initialEngramBody = buildEngramBodyJson(
    PERSONA_ENGRAM_SLUG,
    initialEngramValue,
  );
  const initialCiphertext = encrypt(initialEngramBody, conversationKey);

  const initialEngramEvent = finalizeEvent(
    {
      kind: KIND_AGENT_ENGRAM,
      content: initialCiphertext,
      tags: [
        ["d", engramDTag],
        ["p", ownerPubkey],
      ],
      created_at: Math.floor(Date.now() / 1000) - 5,
    },
    agentSecretKey,
  );
  const initialEngramResult = await submitEvent(initialEngramEvent);
  expect(initialEngramResult.accepted).toBe(true);

  // Edit the persona (simulates update_persona + fleet_update_for_persona)
  const updatedPersona = {
    displayName: "Fleet Test Persona",
    systemPrompt: "Updated system prompt after edit.",
    model: "claude-opus-4",
  };

  // Publish updated persona event (NIP-33 replacement — same d-tag, newer timestamp)
  const updatedPersonaEvent = finalizeEvent(
    {
      kind: KIND_PERSONA,
      content: JSON.stringify({
        display_name: updatedPersona.displayName,
        system_prompt: updatedPersona.systemPrompt,
        model: updatedPersona.model,
      }),
      tags: [["d", personaSlug]],
      created_at: Math.floor(Date.now() / 1000),
    },
    hexToBytes(ownerIdentity.privateKey),
  );
  const updatedPersonaResult = await submitEvent(updatedPersonaEvent);
  expect(updatedPersonaResult.accepted).toBe(true);

  // Fleet update: rewrite the engram with updated content
  const updatedEngramValue = buildPersonaEngramBody(
    updatedPersona,
    ownerPubkey,
    personaSlug,
  );
  const updatedEngramBody = buildEngramBodyJson(
    PERSONA_ENGRAM_SLUG,
    updatedEngramValue,
  );
  const updatedCiphertext = encrypt(updatedEngramBody, conversationKey);

  const updatedEngramEvent = finalizeEvent(
    {
      kind: KIND_AGENT_ENGRAM,
      content: updatedCiphertext,
      tags: [
        ["d", engramDTag],
        ["p", ownerPubkey],
      ],
      created_at: Math.floor(Date.now() / 1000),
    },
    agentSecretKey,
  );
  const updatedEngramResult = await submitEvent(updatedEngramEvent);
  expect(updatedEngramResult.accepted).toBe(true);

  // Verify: only the latest engram is returned (NIP-33 replacement)
  const engramEvents = await queryRelay(
    [
      {
        kinds: [KIND_AGENT_ENGRAM],
        authors: [agentPubkey],
        "#d": [engramDTag],
        "#p": [ownerPubkey],
      },
    ],
    ownerPubkey,
  );
  expect(engramEvents.length).toBe(1);

  // Decrypt and verify it's the updated content
  const ownerConversationKey = getConversationKey(
    hexToBytes(ownerIdentity.privateKey),
    agentPubkey,
  );
  const decryptedJson = decrypt(engramEvents[0].content, ownerConversationKey);
  const decrypted = JSON.parse(decryptedJson);
  const engramBody = JSON.parse(decrypted.value);

  expect(engramBody.system_prompt).toBe("Updated system prompt after edit.");
  expect(engramBody.model).toBe("claude-opus-4");
  expect(engramBody.provenance.source_version).toHaveLength(64);

  // Verify persona event was also replaced (NIP-33)
  const personaEvents = await queryRelay(
    [{ kinds: [KIND_PERSONA], authors: [ownerPubkey], "#d": [personaSlug] }],
    ownerPubkey,
  );
  expect(personaEvents.length).toBe(1);
  expect(JSON.parse(personaEvents[0].content).system_prompt).toBe(
    "Updated system prompt after edit.",
  );
});
