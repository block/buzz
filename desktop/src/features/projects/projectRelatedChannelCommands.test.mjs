import assert from "node:assert/strict";
import test from "node:test";

import { setProjectRelatedChannel } from "./projectRelatedChannelCommands.ts";

const CHANNEL_ID = "11111111-1111-4111-8111-111111111111";
const PROJECT_ADDRESS = `30621:${"a".repeat(64)}:demo`;

function relayEvent({ id, kind, tags }) {
  return {
    id,
    pubkey: "a".repeat(64),
    created_at: 1,
    kind,
    tags,
    content: "",
    sig: "0".repeat(128),
  };
}

test("desired-state write signs and publishes one canonical command", async () => {
  const signed = [];
  let publishes = 0;
  await setProjectRelatedChannel(
    {
      channelId: CHANNEL_ID,
      linked: false,
      projectAddress: PROJECT_ADDRESS,
    },
    {
      signEvent: async (input) => {
        signed.push(input);
        return relayEvent({
          id: String(signed.length).repeat(64),
          kind: input.kind,
          tags: input.tags,
        });
      },
      publishEvent: async () => {
        publishes += 1;
      },
    },
  );
  assert.deepEqual(signed[0].tags, [
    ["a", PROJECT_ADDRESS],
    ["op", "remove"],
    ["d", CHANNEL_ID],
  ]);
  assert.equal(signed.length, 1);
  assert.equal(publishes, 1);
});

test("relay rejections are not retried", async () => {
  let signs = 0;
  let publishes = 0;
  await assert.rejects(
    setProjectRelatedChannel(
      {
        channelId: CHANNEL_ID,
        linked: true,
        projectAddress: PROJECT_ADDRESS,
      },
      {
        signEvent: async (input) => {
          signs += 1;
          return relayEvent({
            id: "f".repeat(64),
            kind: input.kind,
            tags: input.tags,
          });
        },
        publishEvent: async () => {
          publishes += 1;
          throw new Error("relay rejected command");
        },
      },
    ),
    /relay rejected command/,
  );
  assert.equal(signs, 1);
  assert.equal(publishes, 1);
});

for (const [relayMessage, expected] of [
  ["restricted: not a Project administrator", /don't have permission/i],
  ["invalid: target channel is archived", /channel can't be linked/i],
]) {
  test(`maps ${relayMessage.split(":")[0]} relay failures to concise copy`, async () => {
    await assert.rejects(
      setProjectRelatedChannel(
        {
          channelId: CHANNEL_ID,
          linked: true,
          projectAddress: PROJECT_ADDRESS,
        },
        {
          signEvent: async (input) =>
            relayEvent({
              id: "f".repeat(64),
              kind: input.kind,
              tags: input.tags,
            }),
          publishEvent: async () => {
            throw new Error(relayMessage);
          },
        },
      ),
      expected,
    );
  });
}
