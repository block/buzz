// Self-reports need nostr's allow_self_tagging opt-in, or the signer strips the
// reporter's own `p` tag and the relay rejects the report. Only submitReport
// opts in; every other signer caller keeps the stripping default.
import assert from "node:assert/strict";
import test from "node:test";

const signCalls = [];
const author = "ab".repeat(32);
globalThis.window = {
  setTimeout: () => 0,
  clearTimeout: () => {},
  __TAURI_INTERNALS__: {
    invoke: async (command, args) => {
      if (command !== "sign_event") return undefined;
      signCalls.push(args);
      return JSON.stringify({
        id: "00".repeat(32),
        pubkey: author,
        created_at: 0,
        kind: args.kind,
        tags: args.tags,
        content: args.content,
        sig: "00".repeat(64),
      });
    },
  },
};

const { relayClient } = await import("./relayClient.ts");
relayClient.publishEvent = async () => {};
const { submitReport, banMember } = await import("./moderation.ts");

test("submitReport asks the signer to keep a self p tag", async () => {
  signCalls.length = 0;
  await submitReport({
    authorPubkey: author,
    eventId: "cd".repeat(32),
    reportType: "spam",
  });
  assert.equal(signCalls.length, 1);
  assert.equal(signCalls[0].allowSelfTagging, true);
  assert.deepEqual(signCalls[0].tags[0], ["p", author]);
});

test("other moderation events keep the signer's self-tag stripping default", async () => {
  signCalls.length = 0;
  await banMember({ pubkey: author });
  assert.equal(signCalls.length, 1);
  assert.notEqual(signCalls[0].allowSelfTagging, true);
});
