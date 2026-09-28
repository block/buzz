import assert from "node:assert/strict";
import test from "node:test";

import {
  bestieParticipatingThreadInputs,
  bestieThreadRootFromTags,
  parseBestieThreadAddInput,
} from "./bestieThreadDiscover.ts";
import { bestieThreadId } from "./bestieThreadProtocol.ts";

const AGENT =
  "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
const OTHER =
  "112233445566778899aabbccddeeff00112233445566778899aabbccddeeff00";

test("bestieThreadRootFromTags uses root marker for replies", () => {
  assert.equal(
    bestieThreadRootFromTags("reply1", [
      ["e", "root1", "", "root"],
      ["e", "root1", "", "reply"],
    ]),
    "root1",
  );
  assert.equal(bestieThreadRootFromTags("top1", []), "top1");
});

test("bestieParticipatingThreadInputs keeps agent channel threads only", () => {
  const inputs = bestieParticipatingThreadInputs({
    agentPubkey: AGENT,
    excludeChannelIds: ["dm-bestie"],
    items: [
      {
        id: "ev-agent-reply",
        kind: 9,
        pubkey: AGENT,
        content: "Here is my take",
        createdAt: 200,
        channelId: "ch-eng",
        channelName: "eng",
        channelType: "channel",
        tags: [
          ["e", "root-a", "", "root"],
          ["e", "root-a", "", "reply"],
        ],
        category: "agent_activity",
      },
      {
        id: "ev-other",
        kind: 9,
        pubkey: OTHER,
        content: "not agent",
        createdAt: 201,
        channelId: "ch-eng",
        channelName: "eng",
        tags: [],
        category: "agent_activity",
      },
      {
        id: "ev-dm",
        kind: 9,
        pubkey: AGENT,
        content: "dm noise",
        createdAt: 202,
        channelId: "dm-bestie",
        channelName: "Bestie",
        channelType: "dm",
        tags: [],
        category: "agent_activity",
      },
    ],
  });
  assert.equal(inputs.length, 1);
  assert.equal(inputs[0].channelId, "ch-eng");
  assert.equal(inputs[0].rootEventId, "root-a");
  assert.equal(inputs[0].source, "agent");
  assert.equal(inputs[0].preview, "Here is my take");
  assert.equal(
    bestieThreadId(inputs[0].channelId, inputs[0].rootEventId),
    "ch-eng:root-a",
  );
});

test("parseBestieThreadAddInput accepts message links and id pairs", () => {
  const link = parseBestieThreadAddInput(
    "buzz://message?channel=ch-1&id=msg-1&thread=root-1",
  );
  assert.deepEqual(link, {
    channelId: "ch-1",
    messageId: "msg-1",
    rootEventId: "root-1",
  });

  const pair = parseBestieThreadAddInput(
    "580ca78b-9dae-46f3-8854-bd671853ba32 aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899",
  );
  assert.ok(pair);
  assert.equal(pair.channelId, "580ca78b-9dae-46f3-8854-bd671853ba32");
  assert.equal(pair.rootEventId.length, 64);

  assert.equal(parseBestieThreadAddInput("nope"), null);
});
