import assert from "node:assert/strict";
import test from "node:test";
import { ScreenShareReceiver, parseScreenMessage } from "./screenShare.ts";

const channel = "11111111-1111-4111-8111-111111111111";
const session = "abcdef12-1234-4123-8123-123456789abc";
const now = 100_000;
const share = { session_id: session, started_at: now, label: "Display 1" };
const data = "AAAAAWUAAAA=";
function event(message, overrides = {}) {
  return {
    id: "event",
    pubkey: "alice",
    kind: 24811,
    created_at: 100,
    tags: [["h", channel]],
    content: JSON.stringify(message),
    sig: "",
    ...overrides,
  };
}
function batch(sequence, key = true, currentShare = share) {
  return {
    type: "video",
    share: currentShare,
    sequence,
    codec: "avc1.42C028",
    width: 320,
    height: 180,
    frames: Array.from({ length: 8 }, (_, i) => ({
      timestamp: (sequence - 1) * 266_664 + i * 33_333,
      key: key && i === 0,
      data,
    })),
  };
}

test("video parser rejects JPEG snapshots, wrong scopes, expired video, and oversized batches", () => {
  assert.ok(parseScreenMessage(event(batch(1)), channel, now));
  for (const malformed of [
    { tags: [] },
    { tags: [["h", "other"]] },
    {
      tags: [
        ["h", channel],
        ["h", channel],
      ],
    },
    { created_at: 94 },
    { kind: 9 },
    { content: "{" },
    {
      content: JSON.stringify({
        type: "frame",
        share,
        sequence: 1,
        jpeg: "/9j/AAAA",
      }),
    },
    { content: JSON.stringify({ ...batch(1), width: 8192 }) },
    {
      content: JSON.stringify({
        ...batch(1),
        frames: [...batch(1).frames, batch(1).frames[0]],
      }),
    },
    {
      content: JSON.stringify({
        ...batch(1),
        frames: [{ timestamp: 0, key: true, data: "/9j/AAAA" }],
      }),
    },
  ])
    assert.equal(
      parseScreenMessage({ ...event(batch(1)), ...malformed }, channel, now),
      null,
    );
});

test("one transport event preserves all eight timed video frames", () => {
  const receiver = new ScreenShareReceiver(channel);
  assert.equal(receiver.receive(event(batch(1)), now), true);
  const stream = receiver.snapshot()[0];
  const packets = [];
  const unsubscribe = stream.subscribe((packet) => packets.push(packet));
  assert.equal(packets[0].frames.length, 8);
  assert.equal(packets[0].frames[7].timestamp, 233_331);
  assert.equal(receiver.receive(event(batch(2, false)), now), false);
  assert.equal(packets[1].frames.length, 8);
  unsubscribe();
});

test("missing dependency batches wait for the next real keyframe instead of showing corrupt video", () => {
  const receiver = new ScreenShareReceiver(channel);
  receiver.receive(event(batch(1)), now);
  const packets = [];
  receiver.snapshot()[0].subscribe((packet) => packets.push(packet));
  receiver.receive(event(batch(3, false)), now);
  assert.equal(packets.at(-1), null);
  const length = packets.length;
  receiver.receive(event(batch(4, false)), now);
  assert.equal(packets.length, length);
  receiver.receive(event(batch(5)), now);
  assert.equal(packets.at(-1).frames[0].key, true);
});

test("stop and replacement fences prevent an in-flight GOP resurrecting a retired share", () => {
  const receiver = new ScreenShareReceiver(channel);
  receiver.receive(event(batch(1)), now);
  const stream = receiver.snapshot()[0];
  const packets = [];
  stream.subscribe((packet) => packets.push(packet));
  receiver.receive(event({ type: "stopped", share }), now);
  assert.equal(packets.at(-1), null);
  assert.equal(receiver.receive(event(batch(2)), now), false);
  const replacement = {
    ...share,
    session_id: "33333333-3333-4333-8333-333333333333",
    started_at: now + 1,
  };
  assert.equal(receiver.receive(event(batch(1, true, replacement)), now), true);
  assert.equal(receiver.receive(event({ type: "stopped", share }), now), false);
  assert.equal(receiver.snapshot()[0].share.session_id, replacement.session_id);
});

test("disconnect/expiry destroys decoder subscriptions and remains bounded during room churn", () => {
  const receiver = new ScreenShareReceiver(channel);
  receiver.receive(event(batch(1)), now);
  assert.equal(receiver.expire(now + 4999), false);
  assert.equal(receiver.expire(now + 5000), true);
  receiver.clear();
  for (let i = 0; i < 25; i++)
    assert.equal(
      receiver.receive(event(batch(1), { pubkey: `person-${i}` }), now),
      true,
    );
  assert.equal(
    receiver.receive(event(batch(1), { pubkey: "extra" }), now),
    false,
  );
  for (let i = 0; i < 25; i++)
    receiver.receive(
      event({ type: "stopped", share }, { pubkey: `person-${i}` }),
      now,
    );
  assert.equal(
    receiver.receive(event(batch(1), { pubkey: "replacement" }), now),
    true,
  );
  receiver.clear();
  assert.deepEqual(receiver.snapshot(), []);
});

test("UUID casing cannot defeat termination of a video stream", () => {
  const receiver = new ScreenShareReceiver(channel);
  receiver.receive(
    event(batch(1, true, { ...share, session_id: session.toUpperCase() })),
    now,
  );
  assert.equal(receiver.receive(event({ type: "stopped", share }), now), true);
  assert.deepEqual(receiver.snapshot(), []);
});

test("packets from a live sharer surface another sharer's expiry to React", () => {
  const receiver = new ScreenShareReceiver(channel);
  receiver.receive(event(batch(1), { pubkey: "alice" }), now);
  receiver.receive(event(batch(1), { pubkey: "bob" }), now);
  receiver.receive(
    event(batch(2, false), { pubkey: "alice", created_at: 104 }),
    now + 4000,
  );
  assert.equal(
    receiver.receive(
      event(batch(3, false), { pubkey: "alice", created_at: 105 }),
      now + 5000,
    ),
    true,
  );
  assert.deepEqual(
    receiver.snapshot().map((stream) => stream.pubkey),
    ["alice"],
  );
  assert.equal(receiver.expire(now + 5001), false);
});

test("camera and screen tracks from the same signer have independent lifetimes", () => {
  const screens = new ScreenShareReceiver(channel);
  const cameras = new ScreenShareReceiver(channel, 24812);
  const screen = event(batch(1));
  const camera = { ...screen, kind: 24812 };
  assert.equal(screens.receive(screen, now), true);
  assert.equal(cameras.receive(camera, now), true);
  assert.equal(screens.receive(camera, now), false);
  assert.equal(cameras.receive(screen, now), false);
  cameras.receive(event({ type: "stopped", share }, { kind: 24812 }), now);
  assert.equal(cameras.snapshot().length, 0);
  assert.equal(screens.snapshot().length, 1);
});
