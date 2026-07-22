import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import fs from "node:fs";
import { correlateReceipt, loadJson, renderDisabledLaunchAgent, renderWorker, validateManifest } from "./worker.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const manifest = loadJson(join(here, "workers.json"));
// Source checks must be runnable by upstream contributors without the private
// AEON vault mount. Operators can still supply an explicit identity-map path.
const identityMap = loadJson(join(here, "fixtures", "identity-map.json"));

test("six-worker manifest matches the synthetic identity-map contract", () => {
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
    assert.match(argv, /--trusted-inbound-envelope/);
    assert.match(argv, /--no-agent-publisher-credentials/);
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
    assert.match(first.plist, /--no-agent-publisher-credentials/);
    assert.deepEqual(first.rollback, ["launchctl", "bootout", `gui/<uid>/${first.label}`]);
    labels.add(first.label);
  }
  assert.equal(labels.size, 6);
});

test("no-argument LaunchAgent renderer uses the checked-in identity fixture", () => {
  const output = execFileSync(process.execPath, [join(here, "render-launchagents.mjs")], {
    cwd: here,
    encoding: "utf8",
  });
  const rendered = JSON.parse(output);
  assert.equal(rendered.schema, "aeon_disabled_launchagents_v1");
  assert.equal(Object.keys(rendered.artifacts).length, 6);
  for (const worker of manifest.workers) {
    const expected = renderDisabledLaunchAgent(manifest, identityMap, worker.aspect);
    assert.equal(rendered.artifacts[`${expected.label}.plist`], expected.plist);
  }
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
  assert.throws(
    () => renderDisabledLaunchAgent(manifest, identityMap, "nexus", { executablePath: "bin:/usr/bin" }),
    /entries must be absolute/,
  );
  assert.throws(
    () => renderDisabledLaunchAgent(manifest, identityMap, "nexus", { launcherPath: "usr/bin/env" }),
    /must be absolute/,
  );
  assert.throws(
    () => renderDisabledLaunchAgent(manifest, identityMap, "nexus", { stdoutPath: "relative.log" }),
    /must be absolute/,
  );
  assert.throws(
    () => renderDisabledLaunchAgent(manifest, identityMap, "nexus", { agentCommandPrefixArgs: ["relative.mjs"] }),
    /must be absolute/,
  );
  assert.throws(
    () =>
      renderDisabledLaunchAgent(manifest, identityMap, "nexus", {
        agentCommandPrefixArgs: ["/immutable/a/openclaw.mjs", "/immutable/b/openclaw.mjs"],
      }),
    /exactly one/,
  );
  assert.throws(
    () => renderDisabledLaunchAgent(manifest, identityMap, "nexus", { openclawStateDir: "/state" }),
    /must be supplied together/,
  );
  assert.throws(
    () => renderDisabledLaunchAgent(manifest, identityMap, "nexus", {
      openclawConfigPath: "",
      openclawStateDir: "/state",
    }),
    /must be absolute/,
  );
});

test("Fleet can bind the immutable Node runtime without changing disabled previews", () => {
  const defaultRendered = renderDisabledLaunchAgent(manifest, identityMap, "nexus");
  assert.equal(
    defaultRendered.plist,
    fs.readFileSync(join(here, "launchagents", "org.aeon.buzz-acp.nexus.plist"), "utf8"),
  );
  const rendered = renderDisabledLaunchAgent(manifest, identityMap, "nexus", {
    executablePath: "/owned/service-runtime/bin:/usr/bin:/bin",
    openclawConfigPath: "/owned/state/openclaw.json",
    openclawStateDir: "/owned/state",
  });
  assert.match(
    rendered.plist,
    /<key>PATH<\/key><string>\/owned\/service-runtime\/bin:\/usr\/bin:\/bin<\/string>/,
  );
  assert.match(rendered.plist, /<key>OPENCLAW_CONFIG_PATH<\/key><string>\/owned\/state\/openclaw.json<\/string>/);
  assert.match(rendered.plist, /<key>OPENCLAW_STATE_DIR<\/key><string>\/owned\/state<\/string>/);
});

test("Fleet can use a system launcher while retaining the exact Buzz binary", () => {
  const rendered = renderDisabledLaunchAgent(manifest, identityMap, "nexus", {
    buzzAcpPath: "/owned/bin/buzz-acp",
    launcherPath: "/usr/bin/env",
  });
  assert.deepEqual(rendered.argv.slice(0, 2), ["/usr/bin/env", "/owned/bin/buzz-acp"]);
  assert.match(
    rendered.plist,
    /<array>\n    <string>\/usr\/bin\/env<\/string>\n    <string>\/owned\/bin\/buzz-acp<\/string>/,
  );
});

test("Fleet can keep launchd-owned paths local while Buzz reads its canonical config", () => {
  const rendered = renderDisabledLaunchAgent(manifest, identityMap, "nexus", {
    workingDirectory: "/Users/operator",
    privateKeyFile: "/Users/operator/Library/Application Support/AEON/secrets/nexus.sk",
    configPath: "/Volumes/AEON/Projects/buzz/deploy/local/aeon-aspects/config/nexus.toml",
    stdoutPath: "/Users/operator/Library/Logs/AEON/nexus.buzz-acp.log",
    stderrPath: "/Users/operator/Library/Logs/AEON/nexus.buzz-acp.err.log",
  });
  assert.equal(
    rendered.argv[rendered.argv.indexOf("--config") + 1],
    "/Volumes/AEON/Projects/buzz/deploy/local/aeon-aspects/config/nexus.toml",
  );
  assert.equal(
    rendered.argv[rendered.argv.indexOf("--private-key-file") + 1],
    "/Users/operator/Library/Application Support/AEON/secrets/nexus.sk",
  );
  assert.match(rendered.plist, /<key>WorkingDirectory<\/key><string>\/Users\/operator<\/string>/);
  assert.match(rendered.plist, /<key>StandardOutPath<\/key><string>\/Users\/operator\/Library\/Logs\/AEON\/nexus\.buzz-acp\.log<\/string>/);
});

test("Fleet can launch immutable OpenClaw through a local Node identity", () => {
  const rendered = renderDisabledLaunchAgent(manifest, identityMap, "nexus", {
    openclawPath: "/owned/bin/openclaw",
    agentCommandPrefixArgs: ["/immutable/generation/openclaw.mjs"],
  });
  assert.equal(rendered.argv[rendered.argv.indexOf("--agent-command") + 1], "/owned/bin/openclaw");
  assert.equal(
    rendered.argv[rendered.argv.indexOf("--agent-args") + 1],
    "/immutable/generation/openclaw.mjs,acp,--session,agent:main:buzz-private,--require-existing,--token-file,/REQUIRES_FLEET/owned-token-file,--url,ws://127.0.0.1:18806,--provenance,meta+receipt,--no-prefix-cwd",
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
