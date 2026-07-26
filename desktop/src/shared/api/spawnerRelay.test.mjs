import assert from "node:assert/strict";
import test from "node:test";

import {
  parseSpawnerAnnouncement,
  parseSpawnerStatus,
  specSlugFromEvent,
} from "./spawnerRelay.ts";

const statusEvent = (content, tags = [["d", "fizz-prod"]]) => ({
  id: "e".repeat(64),
  pubkey: "a".repeat(64),
  created_at: 1_700_000_000,
  kind: 30179,
  tags,
  content,
  sig: "f".repeat(128),
});

test("parsesARunningStatusFromTheSnakeCaseWire", () => {
  const status = parseSpawnerStatus(
    JSON.stringify({
      phase: "running",
      agent_pubkey: "b".repeat(64),
      spec_hash: "abc123",
      restart_count: 0,
    }),
  );

  assert.deepEqual(status, {
    phase: "running",
    agentPubkey: "b".repeat(64),
    specHash: "abc123",
    error: undefined,
    restartCount: 0,
  });
});

test("parsesAFailedStatusWithItsError", () => {
  const status = parseSpawnerStatus(
    JSON.stringify({
      phase: "failed",
      error: "image pull failed",
      restart_count: 3,
    }),
  );

  assert.equal(status.phase, "failed");
  assert.equal(status.error, "image pull failed");
  assert.equal(status.restartCount, 3);
});

test("restartCountDefaultsToZeroWhenOmitted", () => {
  // The Rust projection skips restart_count when it is 0, so a healthy agent's
  // status has no such field. Defaulting to 0 keeps the UI from rendering
  // "undefined restarts".
  const status = parseSpawnerStatus(JSON.stringify({ phase: "running" }));

  assert.equal(status.restartCount, 0);
});

test("rejectsAnUnknownPhaseRatherThanTrustingIt", () => {
  // An unrecognized phase means this client is older than the spawner. Showing
  // nothing is safer than rendering a phase whose meaning we do not know.
  assert.equal(
    parseSpawnerStatus(JSON.stringify({ phase: "definitely-not-a-phase" })),
    null,
  );
});

test("rejectsMalformedOrEmptyContent", () => {
  assert.equal(parseSpawnerStatus(""), null);
  assert.equal(parseSpawnerStatus("   "), null);
  assert.equal(parseSpawnerStatus("not json at all"), null);
  assert.equal(parseSpawnerStatus("{}"), null);
});

test("readsTheSpecSlugFromTheDTag", () => {
  assert.equal(specSlugFromEvent(statusEvent("{}")), "fizz-prod");
});

test("returnsNullWhenTheDTagIsMissingOrValueless", () => {
  assert.equal(specSlugFromEvent(statusEvent("{}", [])), null);
  assert.equal(specSlugFromEvent(statusEvent("{}", [["p", "x"]])), null);
  // A bare ["d"] carries no slug and must not read as an empty-string slug.
  assert.equal(specSlugFromEvent(statusEvent("{}", [["d"]])), null);
});

const announcementEvent = (content) => ({
  id: "a".repeat(64),
  pubkey: "b".repeat(64),
  created_at: 1_700_000_000,
  kind: 10180,
  tags: [],
  content: typeof content === "string" ? content : JSON.stringify(content),
  sig: "f".repeat(128),
});

test("takesTheAnnouncingPubkeyFromTheEnvelopeNotTheContent", () => {
  // A spawner could otherwise claim to be a different pubkey and get itself
  // connected in place of the real one.
  const a = parseSpawnerAnnouncement(
    announcementEvent({
      name: "prod-vps",
      max_agents: 16,
      agents_running: 3,
      pubkey: "c".repeat(64),
    }),
  );

  assert.equal(a.pubkey, "b".repeat(64));
  assert.equal(a.name, "prod-vps");
  assert.equal(a.maxAgents, 16);
  assert.equal(a.agentsRunning, 3);
});

test("mapsSnakeCaseCapacityAndImageFields", () => {
  const a = parseSpawnerAnnouncement(
    announcementEvent({
      name: "gpu-box",
      description: "4090, Helsinki",
      agent_image: "ghcr.io/block/buzz-acp:main",
      max_agents: 4,
      agents_running: 0,
      max_cpu_millis: 8000,
      max_memory_mib: 32768,
    }),
  );

  assert.equal(a.description, "4090, Helsinki");
  assert.equal(a.agentImage, "ghcr.io/block/buzz-acp:main");
  assert.equal(a.maxCpuMillis, 8000);
  assert.equal(a.maxMemoryMib, 32768);
});

test("rejectsANamelessAnnouncement", () => {
  // A blank row in a spawner picker is worse than no row.
  assert.equal(
    parseSpawnerAnnouncement(
      announcementEvent({ name: "   ", max_agents: 1, agents_running: 0 }),
    ),
    null,
  );
  assert.equal(
    parseSpawnerAnnouncement(announcementEvent({ max_agents: 1 })),
    null,
  );
});

test("rejectsMalformedAnnouncementContent", () => {
  assert.equal(parseSpawnerAnnouncement(announcementEvent("")), null);
  assert.equal(parseSpawnerAnnouncement(announcementEvent("not json")), null);
});

test("defaultsMissingCapacityToZeroRatherThanUndefined", () => {
  // Rendered as "x/y agents"; undefined would surface to the user as text.
  const a = parseSpawnerAnnouncement(announcementEvent({ name: "vps" }));
  assert.equal(a.maxAgents, 0);
  assert.equal(a.agentsRunning, 0);
});

test("emptyStatusContentParsesAsNullSoTombstonesAreDistinguishable", () => {
  // The store relies on this: empty content means "agent deleted", and must be
  // told apart from malformed content, which means "ignore this event".
  assert.equal(parseSpawnerStatus(""), null);
  assert.equal(parseSpawnerStatus("   "), null);
});
