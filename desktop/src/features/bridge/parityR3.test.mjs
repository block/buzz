import test from "node:test";
import assert from "node:assert/strict";

const PK = "ab".repeat(32);
const agents = () => [{ pubkey: PK, name: "A", status: "running" }];

test("directCmd: permission prompts are listed redacted and answered with validated ids", async () => {
  const { handleCmd, CMD_READS, CMD_WRITES } = await import("./directCmd.ts");
  assert.ok(CMD_READS.includes("list_permission_prompts"));
  assert.ok(CMD_WRITES.includes("respond_permission_prompt") && CMD_WRITES.includes("add_team_from_catalog"));
  const calls = [];
  const inv = async (name, args) => {
    calls.push([name, args]);
    if (name === "list_managed_agents") return agents();
    if (name === "list_permission_prompts")
      return [{ id: "1-2", title: "Run export GITHUB_TOKEN=ghp_abcdefghijklmnop1234", kind: "execute", detail: "Authorization: Bearer abcdefghijklmnop", createdAt: 3, options: [{ optionId: "a", name: "Allow", kind: "allow_once" }] }];
    return null;
  };
  const list = await handleCmd("list_permission_prompts", { pubkey: PK }, { invokeRaw: inv });
  assert.equal(list.length, 1);
  assert.ok(!list[0].title.includes("ghp_abcdefghijklmnop1234"), list[0].title);
  assert.ok(!list[0].detail.includes("abcdefghijklmnop"), list[0].detail);
  await handleCmd("respond_permission_prompt", { pubkey: PK, requestId: "1-2", optionId: "a" }, { invokeRaw: inv });
  assert.deepEqual(calls.at(-1), ["respond_permission_prompt", { pubkey: PK, requestId: "1-2", optionId: "a" }]);
  await assert.rejects(handleCmd("respond_permission_prompt", { pubkey: PK, requestId: "../x", optionId: "a" }, { invokeRaw: inv }), /Invalid request/);
  await assert.rejects(handleCmd("list_permission_prompts", { pubkey: "cd".repeat(32) }, { invokeRaw: inv }), /isn't on your desktop/);
});

test("directCmd: add_team_from_catalog passes only the catalog coordinate and returns display fields", async () => {
  const { handleCmd } = await import("./directCmd.ts");
  let seen;
  const inv = async (name, args) => {
    seen = [name, args];
    return { alreadyPresent: false, team: { id: "t1", name: "Team", description: "d", persona_ids: ["p"], is_builtin: false, shared: true, instructions: "secret-ish" } };
  };
  const r = await handleCmd("add_team_from_catalog", { input: { ownerPubkey: PK.toUpperCase(), teamDTag: "team-1", eventId: "cd".repeat(32), extra: 1 } }, { invokeRaw: inv });
  assert.deepEqual(seen, ["add_team_from_catalog", { input: { ownerPubkey: PK, teamDTag: "team-1", eventId: "cd".repeat(32) } }]);
  assert.equal(r.team.name, "Team");
  assert.equal(r.team.instructions, undefined);
  await assert.rejects(handleCmd("add_team_from_catalog", { input: { ownerPubkey: "x", teamDTag: "t", eventId: "cd".repeat(32) } }, { invokeRaw: inv }), /Invalid owner/);
});

