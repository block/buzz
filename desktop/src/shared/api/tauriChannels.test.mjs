import assert from "node:assert/strict";
import test from "node:test";

const { fromRawChannel, fromRawChannelDetail } = await import(
  "./tauriChannels.ts"
);

function rawChannel(overrides = {}) {
  return {
    id: "chan-1",
    name: "General",
    channel_type: "stream",
    visibility: "open",
    description: "",
    topic: null,
    purpose: null,
    member_count: 3,
    member_pubkeys: ["a".repeat(64), "b".repeat(64)],
    last_message_at: null,
    archived_at: null,
    participants: ["Alice", "Bob"],
    participant_pubkeys: ["a".repeat(64), "b".repeat(64)],
    is_member: true,
    ttl_seconds: null,
    ttl_deadline: null,
    ...overrides,
  };
}

function rawDetail(overrides = {}) {
  return {
    ...rawChannel(overrides),
    created_by: "a".repeat(64),
    created_at: "2026-01-01T00:00:00.000Z",
    updated_at: "2026-01-01T00:00:00.000Z",
    topic_set_by: null,
    topic_set_at: null,
    purpose_set_by: null,
    purpose_set_at: null,
    topic_required: false,
    max_members: null,
    nip29_group_id: null,
  };
}

test("fromRawChannel maps participants and participantPubkeys", () => {
  const channel = fromRawChannel(rawChannel());
  assert.deepEqual(channel.participants, ["Alice", "Bob"]);
  assert.deepEqual(channel.participantPubkeys, [
    "a".repeat(64),
    "b".repeat(64),
  ]);
});

test("fromRawChannel defaults missing participants to empty arrays", () => {
  const channel = fromRawChannel(
    rawChannel({ participants: undefined, participant_pubkeys: undefined }),
  );
  assert.deepEqual(
    channel.participants,
    [],
    "missing participants must default to []",
  );
  assert.deepEqual(
    channel.participantPubkeys,
    [],
    "missing participantPubkeys must default to []",
  );
});

test("fromRawChannelDetail defaults missing participants to empty arrays", () => {
  const detail = fromRawChannelDetail(
    rawDetail({ participants: undefined, participant_pubkeys: undefined }),
  );
  assert.deepEqual(detail.participants, []);
  assert.deepEqual(detail.participantPubkeys, []);
});
