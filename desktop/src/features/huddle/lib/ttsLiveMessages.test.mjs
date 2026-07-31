import assert from "node:assert/strict";
import test from "node:test";

import {
  classifySpeakableAgentText,
  createInitialMembershipGate,
  createLatestStateGate,
  createOrderedSpeaker,
  routeLiveAgentText,
} from "./ttsLiveMessages.ts";

const agents = new Set(["agent"]);
const CHANNEL = "active-huddle";
const base = {
  id: "1",
  kind: 9,
  pubkey: "agent",
  content: "Hello there",
  tags: [["h", CHANNEL]],
};
const speakableText = (event, selfPubkey = "human") =>
  classifySpeakableAgentText(event, agents, selfPubkey, CHANNEL).text;

test("speaks only new agent-authored text message events", () => {
  assert.equal(speakableText(base), "Hello there");
  assert.equal(
    speakableText({ ...base, kind: 40002 }),
    "Hello there",
    "managed stream-message-v2 replies are spoken",
  );
  assert.equal(
    speakableText({ ...base, kind: 7 }),
    null,
    "reactions and other event kinds are excluded",
  );
  assert.equal(
    speakableText({ ...base, kind: 10 }),
    null,
    "edits and status events are excluded",
  );
  assert.equal(
    speakableText({ ...base, pubkey: "human" }),
    null,
    "human-authored messages are excluded",
  );
  assert.equal(
    speakableText({ ...base, content: " " }),
    null,
    "empty and non-text content are excluded",
  );
  assert.equal(
    speakableText({ ...base, content: "K" }),
    "K",
    "one-character agent text remains speakable",
  );
  assert.equal(
    speakableText({ ...base, content: "[System] tool started" }),
    null,
    "legacy system rows are excluded",
  );
  assert.equal(
    speakableText({ ...base, tags: [["h", "another-huddle"]] }),
    null,
    "messages for another huddle are excluded",
  );
});

test("routes managed stream-message-v2 through membership and enabled ordering", async () => {
  const invoked = [];
  const speaker = createOrderedSpeaker(async (text, routeId) => {
    invoked.push({ text, routeId });
  }, assert.fail);

  assert.equal(
    routeLiveAgentText(
      { ...base, kind: 40002 },
      agents,
      "human",
      CHANNEL,
      77,
      speaker.enqueue,
    ),
    "queued",
  );
  assert.equal(
    routeLiveAgentText(
      { ...base, kind: 7 },
      agents,
      "human",
      CHANNEL,
      78,
      speaker.enqueue,
    ),
    "unsupported_kind",
  );
  assert.equal(
    routeLiveAgentText(
      { ...base, tags: [["h", "wrong"]] },
      agents,
      "human",
      CHANNEL,
      79,
      speaker.enqueue,
    ),
    "h_tag_mismatch",
  );
  assert.equal(
    routeLiveAgentText(
      { ...base, pubkey: "human" },
      agents,
      "human",
      CHANNEL,
      80,
      speaker.enqueue,
    ),
    "author_not_agent",
  );
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(invoked, [{ text: "Hello there", routeId: 77 }]);
});

test("strips attachment markup and skips attachment-only events", () => {
  const url = "https://cdn.example/voice.png";
  const tags = [...base.tags, ["imeta", `url ${url}`, "m image/png"]];
  assert.equal(
    speakableText({ ...base, content: `![image](${url})`, tags }),
    null,
  );
  assert.equal(
    speakableText({
      ...base,
      content: `Here is the diagram.\n\n![image](${url})`,
      tags,
    }),
    "Here is the diagram.",
  );
  assert.equal(
    speakableText({ ...base, content: `||\n![image](${url})\n||`, tags }),
    null,
  );
});

test("queues agent messages in live thread arrival order", async () => {
  const spoken = [];
  let releaseFirst;
  const firstBlocked = new Promise((resolve) => {
    releaseFirst = resolve;
  });
  const speaker = createOrderedSpeaker(async (text, routeId) => {
    if (text === "first") await firstBlocked;
    spoken.push([text, routeId]);
  }, assert.fail);

  speaker.enqueue("first", 41);
  speaker.enqueue("second", 42);
  await Promise.resolve();
  assert.deepEqual(spoken, []);
  releaseFirst();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(spoken, [
    ["first", 41],
    ["second", 42],
  ]);
});

test("disabling cancels queued speech and rejects new messages until enabled", async () => {
  const invoked = [];
  const dropped = [];
  let releaseFirst;
  const firstBlocked = new Promise((resolve) => {
    releaseFirst = resolve;
  });
  const speaker = createOrderedSpeaker(
    async (text) => {
      invoked.push(text);
      if (text === "first") await firstBlocked;
    },
    assert.fail,
    true,
    (routeId, reason) => dropped.push([routeId, reason]),
  );

  speaker.enqueue("first", 51);
  speaker.enqueue("queued-before-off", 52);
  await Promise.resolve();
  speaker.setEnabled(false);
  speaker.enqueue("while-off");
  releaseFirst();
  await new Promise((resolve) => setTimeout(resolve, 0));
  speaker.setEnabled(true);
  speaker.enqueue("after-on");
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(invoked, ["first", "after-on"]);
  assert.deepEqual(dropped, [[52, "disabled"]]);
});

test("does not speak before the native enabled state is known", async () => {
  const invoked = [];
  const speaker = createOrderedSpeaker(
    async (text) => invoked.push(text),
    assert.fail,
    false,
  );

  speaker.enqueue("before-state");
  await Promise.resolve();
  speaker.setEnabled(true);
  speaker.enqueue("after-state");
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(invoked, ["after-state"]);
});

test("a live TTS state event supersedes a delayed bootstrap result", () => {
  const applied = [];
  const gate = createLatestStateGate((enabled) => applied.push(enabled));
  const applyBootstrap = gate.beginSnapshot();

  gate.applyEvent(false);
  applyBootstrap(true);

  assert.deepEqual(applied, [false]);
});

test("buffers initial live events until membership resolves in order", () => {
  const delivered = [];
  const gate = createInitialMembershipGate((event) => delivered.push(event));
  gate.push("first");
  gate.push("second");
  assert.deepEqual(delivered, []);
  gate.succeed();
  gate.push("third");
  assert.deepEqual(delivered, ["first", "second", "third"]);
});

test("drops the initial buffer fail-closed when membership lookup fails", () => {
  const delivered = [];
  const dropped = [];
  const gate = createInitialMembershipGate(
    (event) => delivered.push(event),
    (event) => dropped.push(event),
  );
  gate.push("unverified");
  gate.fail();
  gate.push("after-failure");
  assert.deepEqual(delivered, ["after-failure"]);
  assert.deepEqual(dropped, ["unverified"]);
});
