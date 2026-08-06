import assert from "node:assert/strict";
import test from "node:test";

import { formatArtilleryChannelMessage } from "./channelEvent.ts";
import {
  buildLiveAgentMovePrompt,
  createManagedArtilleryAgent,
  parseLiveAgentMove,
} from "./liveAgentAdapter.ts";
import { createMockArtilleryMatch } from "./mockAgents.ts";
import {
  ArtilleryAgentTimeoutError,
  createArtilleryChannelEnvelope,
} from "./referee.ts";

const STATE = {
  id: "live-test",
  turn: 1,
  activeSide: "red",
  health: { red: 100, blue: 100 },
  wind: 0,
};

function event({ content, pubkey = "agent-pubkey", kind = 9, tags = [] }) {
  return {
    id: crypto.randomUUID(),
    pubkey,
    created_at: Math.floor(Date.now() / 1_000),
    kind,
    tags,
    content,
    sig: "test-signature",
  };
}

test("builds a strict correlated prompt and parses only its JSON response", () => {
  const prompt = buildLiveAgentMovePrompt(STATE, "request-123");
  assert.match(prompt, /request request-123/);
  assert.match(prompt, /Reply with only this JSON shape/);

  const content = JSON.stringify({
    requestId: "request-123",
    angle: 48,
    power: 76,
    weapon: "pulse-shell",
  });
  assert.deepEqual(parseLiveAgentMove(content, "request-123"), {
    requestId: "request-123",
    angle: 48,
    power: 76,
    weapon: "pulse-shell",
  });
  assert.equal(parseLiveAgentMove(content, "another-request"), null);
  assert.equal(
    parseLiveAgentMove(`\`\`\`json\n${content}\n\`\`\``, "request-123"),
    null,
  );
});

test("accepts only the selected agent's correlated channel reply and cleans up", async () => {
  let onEvent;
  let unsubscribed = false;
  let sentMentionPubkeys;
  const agent = createManagedArtilleryAgent({
    agent: { pubkey: "agent-pubkey", name: "Live Red" },
    channelId: "channel-1",
    responseTimeoutMs: 100,
    side: "red",
    dependencies: {
      subscribe: async (_channelId, callback) => {
        onEvent = callback;
        return async () => {
          unsubscribed = true;
        };
      },
      sendPrompt: async (_channelId, prompt, mentionPubkeys) => {
        sentMentionPubkeys = mentionPubkeys;
        const requestId = prompt.match(/request ([^\n]+)/)?.[1];
        onEvent(
          event({
            content: JSON.stringify({
              requestId,
              angle: 40,
              power: 60,
              weapon: "pulse-shell",
            }),
            pubkey: "somebody-else",
          }),
        );
        queueMicrotask(() => {
          onEvent(
            event({
              content: JSON.stringify({
                requestId,
                angle: 52,
                power: 81,
                weapon: "pulse-shell",
              }),
              kind: 40_002,
            }),
          );
        });
        return { eventId: "prompt-event-1" };
      },
    },
  });

  const move = await agent.decide(STATE);
  assert.equal(typeof move.requestId, "string");
  assert.equal(move.angle, 52);
  assert.equal(move.power, 81);
  assert.equal(move.weapon, "pulse-shell");
  assert.deepEqual(sentMentionPubkeys, ["agent-pubkey"]);
  assert.equal(unsubscribed, true);
});

test("times out an absent live agent and releases the subscription", async () => {
  let unsubscribed = false;
  const agent = createManagedArtilleryAgent({
    agent: { pubkey: "agent-pubkey", name: "Quiet Agent" },
    channelId: "channel-1",
    responseTimeoutMs: 5,
    side: "blue",
    dependencies: {
      subscribe: async () => async () => {
        unsubscribed = true;
      },
      sendPrompt: async () => ({ eventId: "prompt-event-1" }),
    },
  });

  await assert.rejects(agent.decide(STATE), ArtilleryAgentTimeoutError);
  assert.equal(unsubscribed, true);
});

test("formats a completed match as an explicit channel summary", async () => {
  const match = await createMockArtilleryMatch();
  const message = formatArtilleryChannelMessage(
    createArtilleryChannelEnvelope(match),
  );

  assert.match(message, /Buzz Artillery · Bumble vs Fizz/);
  assert.match(message, /Winner: \*\*Bumble\*\*/);
  assert.match(message, /buzz\.game\.artillery\.match\.v1/);
});
