import assert from "node:assert/strict";
import test from "node:test";
import { finalizeEvent, getPublicKey } from "nostr-tools/pure";

import {
  buildProjectRelatedChannelChangeTemplate,
  changeProjectRelatedChannels,
  fetchProjectState,
  parseProjectState,
} from "./projectState.ts";

const RELAY_SECRET = new Uint8Array(32).fill(7);
const RELAY_PUBKEY = getPublicKey(RELAY_SECRET);
const OWNER = "a".repeat(64);
const IDENTITY_ID = "b".repeat(64);
const COORDINATE = `30621:${OWNER}:buzz`;
const CHANNEL_A = "11111111-1111-4111-8111-111111111111";
const CHANNEL_B = "22222222-2222-4222-8222-222222222222";

function stateEvent({
  revision = "1",
  identityId = IDENTITY_ID,
  content,
} = {}) {
  return finalizeEvent(
    {
      kind: 30623,
      created_at: 100,
      content:
        content ??
        JSON.stringify({
          v: 1,
          deleted: false,
          project_tags: [
            ["d", "buzz"],
            ["name", "Buzz"],
            ["buzz-related-channel", CHANNEL_A],
          ],
        }),
      tags: [
        ["d", "c".repeat(64)],
        ["a", COORDINATE],
        ["rev", revision],
        ["e", identityId, "", "identity"],
        ["e", "d".repeat(64), "", "change"],
      ],
    },
    RELAY_SECRET,
  );
}

test("parseProjectState accepts a signed exact-coordinate strict v1 projection", () => {
  const state = parseProjectState(stateEvent(), RELAY_PUBKEY, COORDINATE);
  assert.equal(state.revision, "1");
  assert.equal(state.identityEventId, IDENTITY_ID);
  assert.deepEqual(state.projectTags.at(-1), [
    "buzz-related-channel",
    CHANNEL_A,
  ]);
});

test("parseProjectState rejects wrong authors, signatures, revisions, coordinates, and unknown content fields", () => {
  assert.throws(
    () => parseProjectState(stateEvent(), "e".repeat(64), COORDINATE),
    /signed by this relay/,
  );
  const badSignature = JSON.parse(JSON.stringify(stateEvent()));
  badSignature.sig = "0".repeat(128);
  assert.throws(
    () => parseProjectState(badSignature, RELAY_PUBKEY, COORDINATE),
    /signed by this relay/,
  );
  assert.throws(
    () =>
      parseProjectState(
        stateEvent({ revision: "01" }),
        RELAY_PUBKEY,
        COORDINATE,
      ),
    /revision is not canonical/,
  );
  assert.throws(
    () => parseProjectState(stateEvent(), RELAY_PUBKEY, `30621:${OWNER}:other`),
    /requested coordinate/,
  );
  assert.throws(
    () =>
      parseProjectState(
        stateEvent({
          content: JSON.stringify({
            v: 1,
            deleted: false,
            project_tags: [],
            extra: true,
          }),
        }),
        RELAY_PUBKEY,
        COORDINATE,
      ),
    /strict version 1/,
  );
});

test("buildProjectRelatedChannelChangeTemplate emits the exact sorted kind:47010 command", () => {
  assert.deepEqual(
    buildProjectRelatedChannelChangeTemplate(
      COORDINATE,
      { add: [CHANNEL_B, CHANNEL_A], remove: [] },
      "7",
    ),
    {
      kind: 47010,
      tags: [
        ["a", COORDINATE],
        ["expected-revision", "7"],
      ],
      content: JSON.stringify({
        v: 1,
        patch: {
          related_channels: { add: [CHANNEL_A, CHANNEL_B], remove: [] },
        },
      }),
    },
  );
});

test("fetchProjectState distinguishes absence from a present untrusted projection", async () => {
  const deps = {
    getRelayPubkey: async () => RELAY_PUBKEY,
    fetchEvents: async () => [],
  };
  assert.equal(await fetchProjectState(COORDINATE, deps), null);

  await assert.rejects(
    fetchProjectState(COORDINATE, {
      ...deps,
      fetchEvents: async () => [{ ...stateEvent(), content: "{}" }],
    }),
    /untrusted or unsupported/,
  );
});

test("changeProjectRelatedChannels refetches and retries one revision conflict", async () => {
  const states = [stateEvent({ revision: "4" }), stateEvent({ revision: "5" })];
  const signedTemplates = [];
  let fetchCount = 0;
  let publishCount = 0;
  await changeProjectRelatedChannels(
    { projectAddress: COORDINATE },
    { add: [CHANNEL_B], remove: [] },
    {
      getRelayPubkey: async () => RELAY_PUBKEY,
      fetchEvents: async () => [states[fetchCount++]],
      signEvent: async (template) => {
        signedTemplates.push(template);
        return stateEvent();
      },
      publishEvent: async () => {
        publishCount += 1;
        if (publishCount === 1) {
          throw new Error("conflict: Project revision is 5");
        }
      },
    },
  );

  assert.equal(fetchCount, 2);
  assert.equal(publishCount, 2);
  assert.deepEqual(
    signedTemplates.map((template) => template.tags[1][1]),
    ["4", "5"],
  );
});
