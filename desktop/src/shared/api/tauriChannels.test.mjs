import assert from "node:assert/strict";
import { test } from "node:test";

const { fromRawChannel } = await import("@/shared/api/tauriChannels.ts");

function rawChannel(overrides = {}) {
  return {
    id: "channel-1",
    name: "DM",
    channel_type: "dm",
    visibility: "private",
    description: "",
    topic: null,
    purpose: null,
    member_count: 2,
    member_pubkeys: ["a", "b"],
    last_message_at: null,
    archived_at: null,
    participants: [],
    participant_pubkeys: ["a", "b"],
    ttl_seconds: null,
    ttl_deadline: null,
    ...overrides,
  };
}

test("fromRawChannel defaults participantPubkeys to [] when the relay omits it", () => {
  const raw = rawChannel();
  delete raw.participant_pubkeys;

  const channel = fromRawChannel(raw);

  assert.deepEqual(channel.participantPubkeys, []);
});

test("fromRawChannel preserves participantPubkeys when present", () => {
  const channel = fromRawChannel(rawChannel({ participant_pubkeys: ["a", "b"] }));

  assert.deepEqual(channel.participantPubkeys, ["a", "b"]);
});
