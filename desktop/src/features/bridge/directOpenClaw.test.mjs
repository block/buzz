import assert from "node:assert/strict";
import test from "node:test";
import { handleCmd, checkGitArgv, CMD_NAMES } from "./directCmd.ts";

const calls = [];
const inv = async (cmd, args) => {
  calls.push([cmd, args]);
  if (args?.name === "list_directory")
    return { isError: false, result: { content: [{ type: "text", text: JSON.stringify({ entries: [{ path: "a/README.md" }, { path: "a/.env" }, { path: "a/k.pem" }] }) }] } };
  if (args?.name === "exec") return { isError: false, result: { content: [{ type: "text", text: JSON.stringify({ stdout: "https://bob:ghp_abcdefghijklmnop1234@github.com/x.git\nAPI_KEY=zzz", exitCode: 0 }) }] } };
  return { isError: false, result: { content: [{ type: "text", text: "ok" }] } };
};
const run = (name, args) => handleCmd("call_openclaw_workspace_mcp_tool", { name, arguments: args }, { invokeRaw: inv });

test("proxied over Direct", () => {
  assert.ok(CMD_NAMES.includes("call_openclaw_workspace_mcp_tool"));
  assert.ok(CMD_NAMES.includes("refresh_openclaw_workspace_mcp"));
});
test("only read tools", async () => {
  await assert.rejects(run("write_file", { path: "a", content: "x" }));
  await assert.rejects(run("delete_path", { path: "a" }));
});
test("git allowlist", () => {
  assert.deepEqual(checkGitArgv(["git", "log", "-n", "5", "HEAD"]), ["git", "log", "-n", "5", "HEAD"]);
  checkGitArgv(["git", "worktree", "list", "--porcelain"]);
  checkGitArgv(["git", "remote", "get-url", "origin"]);
  checkGitArgv(["git", "branch", "--list"]);
  for (const bad of [["rm", "-rf", "/"], ["git", "push"], ["git", "-c", "x=y", "log"], ["git", "worktree", "add", "x"], ["git", "remote", "add", "o", "u"], ["git", "branch", "--list", "-D", "x"], ["git", "diff", "--output=/tmp/x"], ["git", "log", "--ext-diff"], ["git", "config", "--list"], ["git", "show", "HEAD:.env"], ["sh", "-c", "git log"]])
    assert.throws(() => checkGitArgv(bad), bad.join(" "));
});
test("secrets stripped", async () => {
  const r = await run("exec", { argv: ["git", "remote", "get-url", "origin"], cwd: "/w/a" });
  const text = JSON.stringify(r);
  assert.ok(!text.includes("ghp_abc"));
  assert.ok(!text.includes("bob:"));
  assert.ok(!text.includes("zzz"));
  const l = await run("list_directory", { path: "/w/a", depth: 99, namesOnly: true });
  const lt = JSON.stringify(l);
  assert.ok(lt.includes("README.md") && !lt.includes(".env") && !lt.includes("k.pem"));
  assert.equal(calls.at(-1)[1].arguments.depth, 8);
  await assert.rejects(run("read_file", { path: "/w/a/.env" }));
  await assert.rejects(run("read_file", { path: "/home/u/.ssh/id_ed25519" }));
});
test("gh_pr_list and git identity re-picked", async () => {
  const inv2 = async (cmd) =>
    cmd === "gh_pr_list" ? JSON.stringify([{ number: 3, title: "x", state: "OPEN", url: "u", secret: "s", headRefName: "h", baseRefName: "main" }]) : { name: "JM", email: "j@x", token: "t" };
  const prs = JSON.parse(await handleCmd("gh_pr_list", { repo: "jmfcamp/buzz" }, { invokeRaw: inv2 }));
  assert.equal(prs[0].number, 3);
  assert.ok(!("secret" in prs[0]));
  await assert.rejects(handleCmd("gh_pr_list", { repo: "--help" }, { invokeRaw: inv2 }));
  assert.deepEqual(await handleCmd("get_git_identity", {}, { invokeRaw: inv2 }), { name: "JM", email: "j@x" });
});
