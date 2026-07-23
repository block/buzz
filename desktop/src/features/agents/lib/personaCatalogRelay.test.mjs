import assert from "node:assert/strict";
import test from "node:test";

import {
  catalogPersonasFromPublications,
  catalogPublicationsFromEvents,
  sanitizeCatalogSnapshotBytes,
} from "./personaCatalogRelay.ts";

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);
const SNAPSHOT = {
  url: `https://relay.example/media/${"c".repeat(64)}`,
  sha256: "c".repeat(64),
  size: 512,
  type: "application/json",
  fileName: "reviewer.agent.json",
};

function catalogEvent({
  createdAt,
  id,
  owner = ALICE,
  sourcePersonaId = "reviewer",
  status = "published",
  memoryLevel = "none",
}) {
  const sourceUpdatedAt = `2026-07-23T00:00:0${createdAt}.000Z`;
  const content =
    status === "published"
      ? {
          format: "buzz-persona-catalog",
          version: 1,
          status,
          sourcePersonaId,
          sourceUpdatedAt,
          memoryLevel,
          agent: {
            displayName: "Relay Reviewer",
            avatarUrl: null,
            systemPrompt: "Review changes.",
            runtime: "goose",
            model: "claude",
            provider: null,
          },
          snapshot: SNAPSHOT,
        }
      : {
          format: "buzz-persona-catalog",
          version: 1,
          status,
          sourcePersonaId,
          sourceUpdatedAt,
        };
  return {
    id,
    pubkey: owner,
    created_at: createdAt,
    kind: 30178,
    tags: [
      ["d", sourcePersonaId],
      ["status", status],
      ["source_updated_at", sourceUpdatedAt],
      ...(status === "published" ? [["memory", memoryLevel]] : []),
    ],
    content: JSON.stringify(content),
    sig: "sig",
  };
}

test("a catalog publication from Alice is discoverable by Bob", () => {
  const publications = catalogPublicationsFromEvents([
    catalogEvent({ createdAt: 1, id: "alice-reviewer" }),
  ]);
  const personas = catalogPersonasFromPublications(publications, [], BOB);

  assert.equal(personas.length, 1);
  assert.equal(personas[0].displayName, "Relay Reviewer");
  assert.equal(personas[0].isActive, false);
  assert.equal(personas[0].catalogSource.ownerPubkey, ALICE);
  assert.equal(personas[0].catalogSource.isOwn, false);
  assert.deepEqual(personas[0].catalogSource.snapshot, SNAPSHOT);
});

test("the newest unpublished head removes an older publication from discovery", () => {
  const publications = catalogPublicationsFromEvents([
    catalogEvent({ createdAt: 1, id: "published" }),
    catalogEvent({ createdAt: 2, id: "unpublished", status: "unpublished" }),
  ]);

  assert.equal(publications.length, 1);
  assert.equal(publications[0].status, "unpublished");
  assert.deepEqual(catalogPersonasFromPublications(publications, [], BOB), []);
});

test("catalog coordinates remain independent across authors", () => {
  const publications = catalogPublicationsFromEvents([
    catalogEvent({ createdAt: 1, id: "alice", owner: ALICE }),
    catalogEvent({ createdAt: 1, id: "bob", owner: BOB }),
  ]);

  assert.equal(publications.length, 2);
  assert.equal(
    catalogPersonasFromPublications(publications, [], BOB).length,
    2,
  );
});

test("catalog snapshot sanitization strips secrets and response allowlists", () => {
  const source = {
    format: "buzz-agent-snapshot",
    version: 1,
    definition: {
      name: "Reviewer",
      systemPrompt: "Review changes.",
      runtime: "goose",
      model: "claude",
      provider: "anthropic",
      respondTo: "allowlist",
      respondToAllowlist: [BOB],
      namePool: ["Reviewer"],
      envVars: { ANTHROPIC_API_KEY: "secret" },
      privateKeyNsec: "nsec-secret",
      authTag: "auth-secret",
    },
    profile: { displayName: "Reviewer" },
    memory: { level: "none", entries: [] },
    relayUrl: "wss://private.example",
  };

  const sanitized = JSON.parse(
    new TextDecoder().decode(
      Uint8Array.from(
        sanitizeCatalogSnapshotBytes(
          Array.from(new TextEncoder().encode(JSON.stringify(source))),
          "none",
        ),
      ),
    ),
  );

  assert.equal(sanitized.definition.systemPrompt, "Review changes.");
  assert.equal(sanitized.definition.respondTo, "allowlist");
  assert.equal("respondToAllowlist" in sanitized.definition, false);
  assert.equal("envVars" in sanitized.definition, false);
  assert.equal("privateKeyNsec" in sanitized.definition, false);
  assert.equal("authTag" in sanitized.definition, false);
  assert.equal("relayUrl" in sanitized, false);
});

test("selected core memory survives the public allowlist projection", () => {
  const source = {
    format: "buzz-agent-snapshot",
    version: 1,
    definition: { name: "Reviewer", systemPrompt: "Review changes." },
    profile: { displayName: "Reviewer" },
    memory: {
      level: "core",
      entries: [{ slug: "core", body: "Prefers concise findings." }],
    },
  };
  const sanitized = JSON.parse(
    new TextDecoder().decode(
      Uint8Array.from(
        sanitizeCatalogSnapshotBytes(
          Array.from(new TextEncoder().encode(JSON.stringify(source))),
          "core",
        ),
      ),
    ),
  );

  assert.deepEqual(sanitized.memory, source.memory);
});

test("catalog snapshot sanitization rejects memory beyond the selected level", () => {
  const source = {
    format: "buzz-agent-snapshot",
    version: 1,
    definition: { name: "Reviewer", systemPrompt: "Review changes." },
    profile: { displayName: "Reviewer" },
    memory: {
      level: "core",
      entries: [
        { slug: "core", body: "Public core instructions." },
        { slug: "mem/private", body: "Must not escape a core-only share." },
      ],
    },
  };

  assert.throws(
    () =>
      sanitizeCatalogSnapshotBytes(
        Array.from(new TextEncoder().encode(JSON.stringify(source))),
        "core",
      ),
    /outside the selected sharing level/u,
  );
});
