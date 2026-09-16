import assert from "node:assert/strict";
import test from "node:test";

import {
  channelAgentMembers,
  channelBotMemberPubkeySet,
  channelMemberPubkeySet,
  channelRoleMap,
} from "./rosterDerivations.ts";

const HUMAN = "A".repeat(64);
const BOT_ROLE = "b".repeat(64);
const FLAGGED_AGENT = "c".repeat(64);

function roster() {
  return [
    { pubkey: HUMAN, role: "owner", isAgent: false },
    { pubkey: BOT_ROLE, role: "bot", isAgent: false },
    { pubkey: FLAGGED_AGENT, role: "member", isAgent: true },
  ];
}

test("channelRoleMap lowercases pubkeys and maps roles", () => {
  const map = channelRoleMap(roster());
  assert.equal(map.get(HUMAN.toLowerCase()), "owner");
  assert.equal(map.get(BOT_ROLE), "bot");
  assert.equal(map.size, 3);
});

test("channelAgentMembers keeps bot-role and agent-flagged members", () => {
  const agents = channelAgentMembers(roster());
  assert.deepEqual(
    agents.map((member) => member.pubkey),
    [BOT_ROLE, FLAGGED_AGENT],
  );
});

test("channelMemberPubkeySet normalizes; bot set is role-gated only", () => {
  const members = roster();
  assert.deepEqual(
    [...channelMemberPubkeySet(members)],
    [HUMAN.toLowerCase(), BOT_ROLE, FLAGGED_AGENT],
  );
  // role === "bot" only — an isAgent flag without the role stays out, matching
  // the agent-session filter this set feeds.
  assert.deepEqual([...channelBotMemberPubkeySet(members)], [BOT_ROLE]);
});

test("derivations are cached on roster identity", () => {
  const members = roster();
  assert.equal(channelRoleMap(members), channelRoleMap(members));
  assert.equal(channelAgentMembers(members), channelAgentMembers(members));
  assert.equal(
    channelMemberPubkeySet(members),
    channelMemberPubkeySet(members),
  );
  assert.equal(
    channelBotMemberPubkeySet(members),
    channelBotMemberPubkeySet(members),
  );

  // A new array — even with equal content — recomputes: the cache is keyed on
  // identity, which React Query's structural sharing keeps stable for
  // unchanged rosters.
  assert.notEqual(channelRoleMap(members), channelRoleMap(roster()));
});
