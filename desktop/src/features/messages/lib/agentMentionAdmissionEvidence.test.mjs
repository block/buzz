import assert from "node:assert/strict";
import test from "node:test";
import {
  AgentMentionAuthorizationError,
  revalidateAgentMentionPubkeys,
} from "./agentMentionRevalidation.ts";

const OWNER = "a".repeat(64);
const AGENT = "b".repeat(64);
const HUMAN = "c".repeat(64);

function fixture() {
  let policy = "anyone";
  let calls = 0;
  const options = {
    pubkeys: [HUMAN, AGENT],
    agentPubkeys: new Set([AGENT]),
    currentPubkey: OWNER,
    eligibilityScope: { type: "channel", channelId: "destination" },
    sharedChannelIds: new Set(["destination"]),
    phase: "prepare",
    refetchManagedAgents: async () => ({ data: [], error: null }),
    fetchRelayAgents: async (keys) => {
      calls++;
      assert.deepEqual(keys, [AGENT]);
      return [
        {
          pubkey: AGENT,
          ownerPubkey: OWNER,
          respondTo: policy,
          respondToAllowlist: [],
          channelIds: ["destination"],
        },
      ];
    },
  };
  return {
    options,
    revoke: () => {
      policy = "nobody";
    },
    allow: () => {
      policy = "anyone";
    },
    calls: () => calls,
  };
}

function failure(reason) {
  return (error) => {
    assert.ok(error instanceof AgentMentionAuthorizationError);
    assert.equal(error.reason, reason);
    assert.match(error.message, /[Rr]etry/);
    return true;
  };
}

test("prepare reads fresh policy each time; retry and publication do not reuse success", async () => {
  const f = fixture();
  const originalKeys = [...f.options.pubkeys];
  assert.deepEqual(
    await revalidateAgentMentionPubkeys(f.options),
    originalKeys,
  );
  f.revoke(); // No cache invalidation/refetch outside the action under test.
  await assert.rejects(
    revalidateAgentMentionPubkeys(f.options),
    failure("denied"),
  );
  f.allow();
  assert.deepEqual(
    await revalidateAgentMentionPubkeys(f.options),
    originalKeys,
  );
  f.revoke();
  await assert.rejects(
    revalidateAgentMentionPubkeys({ ...f.options, phase: "publish" }),
    failure("denied"),
  );
  assert.equal(f.calls(), 4);
  assert.deepEqual(f.options.pubkeys, originalKeys);
});

test("missing exact identity with complete evidence is denied", async () => {
  await assert.rejects(
    revalidateAgentMentionPubkeys({
      ...fixture().options,
      fetchRelayAgents: async () => [],
    }),
    failure("denied"),
  );
});

for (const source of [
  "relay",
  "managed-rejection",
  "managed-error",
  "managed-missing",
]) {
  test(`${source}: incomplete evidence is lookup-failed, never a policy claim`, async () => {
    const { options } = fixture();
    options.fetchRelayAgents = async () => [];
    if (source === "relay") {
      options.fetchRelayAgents = async () => {
        throw new Error("offline");
      };
    } else if (source === "managed-rejection") {
      options.refetchManagedAgents = async () => {
        throw new Error("offline");
      };
    } else {
      options.refetchManagedAgents = async () => ({
        data: source === "managed-error" ? [{ pubkey: AGENT }] : undefined,
        error: source === "managed-error" ? new Error("offline") : null,
      });
    }
    await assert.rejects(
      revalidateAgentMentionPubkeys(options),
      failure("lookup-failed"),
    );
  });
}

test("valid independent evidence wins despite unrelated lookup failure", async () => {
  const { options } = fixture();
  assert.deepEqual(
    await revalidateAgentMentionPubkeys({
      ...options,
      refetchManagedAgents: async () => {
        throw new Error("local offline");
      },
    }),
    options.pubkeys,
  );
  assert.deepEqual(
    await revalidateAgentMentionPubkeys({
      ...options,
      refetchManagedAgents: async () => ({
        data: [{ pubkey: AGENT }],
        error: null,
      }),
      fetchRelayAgents: async () => {
        throw new Error("relay offline");
      },
    }),
    options.pubkeys,
  );
});
