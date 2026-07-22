import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import fs from "node:fs";
import { correlateReceipt, loadJson, renderDisabledLaunchAgent, renderWorker, validateManifest } from "./worker.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const manifest = loadJson(join(here, "workers.json"));
const identityMap = loadJson(manifest.identityMap);

test("six-worker manifest matches the AEON identity map", () => {
  const result = validateManifest(manifest, identityMap);
  assert.equal(result.ok, true, result.errors.join("\n"));
  assert.equal(manifest.workers.length, 6);
  assert.match(result.warnings[0], /avatar metadata is absent/);
});

test("every rendered worker is disabled and binds an existing fixed session", () => {
  for (const worker of manifest.workers) {
    const rendered = renderWorker(manifest, identityMap, worker.aspect, "/owned/gateway.token");
    const argv = rendered.args.join(" ");
    assert.equal(rendered.enabled, false);
    assert.equal(rendered.supervisor.runAtLoad, false);
    assert.equal(rendered.supervisor.restartOnFailure, false);
    assert.match(argv, /--no-memory/);
    assert.match(argv, /--no-base-prompt/);
    assert.match(argv, /--respond-to owner-only/);
    assert.match(argv, /--allowed-respond-to owner-only/);
    assert.match(argv, /--agents 1/);
    assert.match(argv, /--dedup queue/);
    assert.match(argv, /--multiple-event-handling queue/);
    assert.match(argv, /--relay-observer/);
    assert.match(argv, /--permission-mode bypass-permissions/);
    assert.match(argv, /--heartbeat-interval 0/);
    assert.match(argv, /--turn-liveness-secs 10/);
    assert.match(argv, /--idle-timeout 900/);
    assert.match(argv, /--max-turn-duration 7200/);
    assert.match(argv, /--context-message-limit 12/);
    assert.match(argv, /--max-turns-per-session 0/);
    assert.match(argv, /--turn-receipts/);
    assert.doesNotMatch(argv, /--no-presence|--no-typing|--no-ignore-self/);
    assert.doesNotMatch(argv, /--mcp-command|--model|--system-prompt|--team-instructions|--initial-message/);
    assert.match(argv, new RegExp(`--expected-gateway-session-key ${worker.sessionKey}`));
    assert.match(argv, new RegExp(`--private-key-file ${identityMap.members[worker.aspect].secret_ref}`));
    assert.match(argv, new RegExp(`--expected-public-key ${worker.pubkey}`));
    assert.match(argv, /--agent-args acp,--session,agent:[a-z]+:buzz-private,--require-existing,--token-file,\/owned\/gateway.token,--url,ws:\/\/127.0.0.1:18806,--provenance,meta\+receipt,--no-prefix-cwd/);
  }
});

test("six deterministic LaunchAgent previews are disabled and secret-free", () => {
  const labels = new Set();
  for (const worker of manifest.workers) {
    const first = renderDisabledLaunchAgent(manifest, identityMap, worker.aspect);
    const second = renderDisabledLaunchAgent(manifest, identityMap, worker.aspect);
    assert.deepEqual(second, first);
    assert.equal(first.runAtLoad, false);
    assert.equal(first.keepAlive, false);
    assert.deepEqual(first.tokenFileContract, {
      absolute: true,
      regular: true,
      symlink: false,
      owner: "current-user",
      mode: "0600",
    });
    assert.match(first.plist, /<key>RunAtLoad<\/key><false\/>/);
    assert.match(first.plist, /<key>KeepAlive<\/key><false\/>/);
    assert.match(first.plist, /\/REQUIRES_FLEET\/immutable-openclaw\/bin\/openclaw/);
    assert.match(first.plist, /\/REQUIRES_FLEET\/owned-token-file/);
    assert.doesNotMatch(first.plist, /nsec1|BUZZ_PRIVATE_KEY=/);
    assert.deepEqual(first.rollback, ["launchctl", "bootout", `gui/<uid>/${first.label}`]);
    labels.add(first.label);
  }
  assert.equal(labels.size, 6);
});

test("LaunchAgent rendering rejects unsafe or relative runtime paths", () => {
  assert.throws(
    () => renderDisabledLaunchAgent(manifest, identityMap, "nexus", { tokenFile: "relative.token" }),
    /must be absolute/,
  );
  assert.throws(
    () => renderDisabledLaunchAgent(manifest, identityMap, "nexus", { openclawPath: "/bad,command" }),
    /forbidden delimiter/,
  );
});

test("worker restart renders the identical require-existing Gateway binding", () => {
  const first = renderWorker(manifest, identityMap, "nexus", "/owned/gateway.token");
  const restarted = renderWorker(manifest, identityMap, "nexus", "/owned/gateway.token");
  assert.deepEqual(restarted, first);
  assert.match(first.args.join(" "), /--session,agent:main:buzz-private,--require-existing/);
});

test("Nexus activation is not coupled to the legacy aeon-buzz bridge", () => {
  const rendered = renderWorker(manifest, identityMap, "nexus");
  const serialized = JSON.stringify(rendered);
  assert.doesNotMatch(serialized, /aeon-buzz/);
  assert.equal(rendered.sessionKey, "agent:main:buzz-private");
});

test("each room config enforces Architect-only private and huddle rules", () => {
  for (const worker of manifest.workers) {
    const source = fs.readFileSync(join(here, "config", `${worker.aspect}.toml`), "utf8");
    assert.match(source, new RegExp(worker.privateChannelId));
    assert.equal((source.match(/kinds = \[9, 40002\]/g) ?? []).length, 2);
    assert.equal((source.match(/require_exact_channel_tag = true/g) ?? []).length, 2);
    assert.match(source, /require_mention = false/);
    assert.match(source, /admit_invited_ephemeral = true/);
    assert.match(source, /require_mention = true/);
    assert.equal((source.match(new RegExp(manifest.buzz.architectPubkey, "g")) ?? []).length, 2);
    assert.doesNotMatch(source, new RegExp(manifest.buzz.conciliumChannelId));
  }
});

test("receipt correlation joins one request, one anchored reply, session, and run", () => {
  assert.deepEqual(
    correlateReceipt({
      triggeringEventIds: ["request-1"],
      replyEvents: [{ eventId: "reply-1", replyTo: "request-1" }],
      sessionKey: "agent:main:buzz-private",
      runId: "run-1"
    }),
    {
      requestEventId: "request-1",
      replyEventId: "reply-1",
      gatewaySessionKey: "agent:main:buzz-private",
      runId: "run-1"
    }
  );
});

test("receipt correlation fails closed on zero or duplicate replies", () => {
  const base = { triggeringEventIds: ["request-1"], sessionKey: "session", runId: "run" };
  assert.throws(() => correlateReceipt({ ...base, replyEvents: [] }), /found 0/);
  assert.throws(
    () => correlateReceipt({ ...base, replyEvents: [
      { eventId: "reply-1", replyTo: "request-1" },
      { eventId: "reply-2", replyTo: "request-1" }
    ] }),
    /found 2/
  );
});
