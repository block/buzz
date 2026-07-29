/**
 * Agent card avatar precedence.
 *
 * The harness-mark fallback exists for provider-backed records only, and only
 * as a last resort: a local record already carries a stamped avatar and must
 * render exactly as it did, and anything a human or the agent itself chose
 * outranks a logo.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { PRESET_LOGOS } from "../../onboarding/ui/RuntimeIcon.tsx";
import { resolveAgentAvatarUrl } from "./agentAvatarUrl.ts";

const CHOSEN = "https://cdn.example/marshall.png";
const PUBLISHED = "https://cdn.example/published.png";

const remoteAgent = {
  backend: { type: "provider", id: "ssh", config: { ssh_host: "vps" } },
  agentCommand: "hermes",
  agentArgs: ["--profile", "marshall", "acp"],
};

/** A local record, whose avatar was stamped from this computer's catalog. */
const localAgent = {
  backend: { type: "local" },
  agentCommand: "claude-agent-acp",
  agentArgs: [],
};

test("a remote record falls back to its pinned harness mark", () => {
  // The bug: the host's catalog entry deliberately carries no avatar url, so
  // this record rendered blank initials while its local twin showed a logo.
  assert.equal(
    resolveAgentAvatarUrl({ agent: remoteAgent }),
    PRESET_LOGOS.hermes,
  );
});

test("a chosen avatar outranks the harness mark", () => {
  assert.equal(
    resolveAgentAvatarUrl({
      agent: remoteAgent,
      personaAvatarUrl: CHOSEN,
      profileAvatarUrl: PUBLISHED,
    }),
    CHOSEN,
  );
  assert.equal(
    resolveAgentAvatarUrl({ agent: remoteAgent, profileAvatarUrl: PUBLISHED }),
    PUBLISHED,
  );
});

test("blank candidates do not outrank anything", () => {
  assert.equal(
    resolveAgentAvatarUrl({
      agent: remoteAgent,
      personaAvatarUrl: "   ",
      profileAvatarUrl: "",
    }),
    PRESET_LOGOS.hermes,
  );
  assert.equal(
    resolveAgentAvatarUrl({
      agent: remoteAgent,
      personaAvatarUrl: null,
      profileAvatarUrl: `  ${PUBLISHED}  `,
    }),
    PUBLISHED,
    "a chosen url is trimmed, as the card's own helper always did",
  );
});

test("a local record gains no fallback", () => {
  // Byte-identical to the previous behavior: whatever the record and profile
  // carry, and initials when they carry nothing.
  assert.equal(resolveAgentAvatarUrl({ agent: localAgent }), null);
  assert.equal(
    resolveAgentAvatarUrl({ agent: localAgent, profileAvatarUrl: PUBLISHED }),
    PUBLISHED,
  );
});

test("the record's own stamp is used before the harness mark", () => {
  // The timeline has no definition to read, so it passes the record's stamped
  // avatar. A local record was stamped at create time and must keep showing
  // exactly that; a remote one was stamped with nothing, which is the gap.
  const STAMPED = "app-avatar://claude";
  assert.equal(
    resolveAgentAvatarUrl({ agent: localAgent, recordAvatarUrl: STAMPED }),
    STAMPED,
  );
  assert.equal(
    resolveAgentAvatarUrl({ agent: remoteAgent, recordAvatarUrl: null }),
    PRESET_LOGOS.hermes,
  );
  assert.equal(
    resolveAgentAvatarUrl({
      agent: remoteAgent,
      profileAvatarUrl: PUBLISHED,
      recordAvatarUrl: STAMPED,
    }),
    PUBLISHED,
    "what the agent published about itself still outranks its stamp",
  );
});

test("a never-spawned persona shows only what its definition carries", () => {
  assert.equal(
    resolveAgentAvatarUrl({ agent: undefined, personaAvatarUrl: CHOSEN }),
    CHOSEN,
  );
  assert.equal(resolveAgentAvatarUrl({ agent: undefined }), null);
});

test("a remote record on an unknown harness earns no mark", () => {
  assert.equal(
    resolveAgentAvatarUrl({
      agent: {
        backend: { type: "provider", id: "ssh", config: {} },
        agentCommand: "acme-brain",
        agentArgs: [],
      },
    }),
    null,
    "initials are honest; a borrowed logo would not be",
  );
});
