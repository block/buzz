import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import fs from "node:fs";
import {
  assertTrustedPublisherContract,
  correlateReceipt,
  evaluateSemanticHealth,
  loadJson,
  renderDisabledLaunchAgent,
  renderPrivateOfficePrompt,
  renderWorker,
  validateManifest,
} from "./worker.mjs";

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
    assert.equal(rendered.supervisor.startOnAppLaunch, true);
    assert.equal(rendered.supervisor.runAtLoad, true);
    assert.equal(rendered.supervisor.restartOnFailure, true);
    assert.match(argv, /--no-memory/);
    assert.match(argv, new RegExp(`--base-prompt-file ${worker.basePromptFile}`));
    assert.doesNotMatch(argv, /--no-base-prompt/);
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

test("one shared contract renders exact trusted publisher prompts for all six offices", () => {
  const template = fs.readFileSync(join(here, "prompts", "private-office.template.md"), "utf8");
  assert.equal((template.match(/\{\{ROOM\}\}/g) ?? []).length, 1);
  assert.equal((template.match(/\{\{REPLY_TOOL\}\}/g) ?? []).length, 1);
  assert.equal((template.match(/\{\{/g) ?? []).length, 2);
  for (const worker of manifest.workers) {
    const rendered = renderWorker(manifest, identityMap, worker.aspect);
    const promptPath = join(here, "..", "..", "..", worker.basePromptFile);
    const prompt = fs.readFileSync(promptPath, "utf8");
    const tool = `buzz_${worker.aspect}_reply`;
    assert.equal(prompt, renderPrivateOfficePrompt(template, worker.aspect));
    assert.match(prompt, new RegExp(`#aspect-${worker.aspect}`));
    assert.match(prompt, new RegExp(`exactly one \`${tool}\``));
    assert.match(prompt, /Plain assistant text is not published to Buzz/);
    assert.doesNotMatch(prompt, /buzz messages send/);
    assert.match(prompt, /Publisher credentials are intentionally withheld/);
    assert.match(prompt, /full existing OpenClaw tool, skill, memory, identity, and session capabilities/);
    assert.match(prompt, /does not restrict any other tool use or capability/);
    assert.equal((prompt.match(new RegExp(tool, "g")) ?? []).length, 1);
    for (const other of manifest.workers) {
      if (other.aspect !== worker.aspect) {
        assert.doesNotMatch(prompt, new RegExp(`buzz_${other.aspect}_reply`));
      }
    }
    assert.doesNotMatch(rendered.args.join(" "), /--no-base-prompt/);
  }
});

test("credential isolation, trusted envelope, receipt, and exact prompt are indivisible", () => {
  for (const worker of manifest.workers) {
    const rendered = renderWorker(manifest, identityMap, worker.aspect);
    assert.doesNotThrow(() =>
      assertTrustedPublisherContract(rendered.args, worker.aspect, worker.basePromptFile),
    );
    for (const required of [
      "--no-agent-publisher-credentials",
      "--trusted-inbound-envelope",
      "--base-prompt-file",
      "--turn-receipts",
    ]) {
      const broken = rendered.args.filter((value) => value !== required);
      assert.throws(
        () => assertTrustedPublisherContract(broken, worker.aspect, worker.basePromptFile),
        new RegExp(required.replaceAll("-", "\\-")),
      );
    }
    const wrongPrompt = [...rendered.args];
    wrongPrompt[wrongPrompt.indexOf("--base-prompt-file") + 1] = "/wrong/prompt.md";
    assert.throws(
      () => assertTrustedPublisherContract(wrongPrompt, worker.aspect, worker.basePromptFile),
      /base prompt drift/,
    );
  }
});

test("no-argument prompt renderer emits the six checked-in generated contracts", () => {
  const output = execFileSync(process.execPath, [join(here, "render-private-office-prompts.mjs")], {
    cwd: here,
    encoding: "utf8",
  });
  const rendered = JSON.parse(output);
  assert.equal(rendered.schema, "aeon_private_office_prompts_v1");
  assert.equal(Object.keys(rendered.artifacts).length, 6);
  for (const worker of manifest.workers) {
    assert.equal(
      rendered.artifacts[`${worker.aspect}-private-office.md`],
      fs.readFileSync(join(here, "prompts", `${worker.aspect}-private-office.md`), "utf8"),
    );
  }
});

test("Nexus uses the uniform publisher contract without changing its fixed session", () => {
  const rendered = renderWorker(manifest, identityMap, "nexus", "/owned/gateway.token");
  assert.match(
    rendered.args.join(" "),
    /--base-prompt-file deploy\/local\/aeon-aspects\/prompts\/nexus-private-office\.md/,
  );
  assert.match(rendered.args.join(" "), /--session,agent:main:buzz-private,--require-existing/);
});

test("six deterministic LaunchAgent previews are disabled and secret-free", () => {
  const labels = new Set();
  for (const worker of manifest.workers) {
    const first = renderDisabledLaunchAgent(manifest, identityMap, worker.aspect);
    const second = renderDisabledLaunchAgent(manifest, identityMap, worker.aspect);
    assert.deepEqual(second, first);
    assert.equal(first.runAtLoad, true);
    assert.equal(first.keepAlive, true);
    assert.deepEqual(first.tokenFileContract, {
      absolute: true,
      regular: true,
      symlink: false,
      owner: "current-user",
      mode: "0600",
    });
    assert.match(first.plist, /<key>RunAtLoad<\/key><true\/>/);
    assert.match(first.plist, /<key>KeepAlive<\/key><true\/>/);
    assert.match(first.plist, /\/REQUIRES_FLEET\/immutable-openclaw\/bin\/openclaw/);
    assert.match(first.plist, /\/REQUIRES_FLEET\/owned-token-file/);
    assert.doesNotMatch(first.plist, /nsec1|BUZZ_PRIVATE_KEY=/);
    assert.match(first.plist, /--no-agent-publisher-credentials/);
    assert.match(first.plist, /--base-prompt-file/);
    assert.match(first.plist, new RegExp(`/Volumes/AEON/Projects/buzz/${worker.basePromptFile}`));
    assert.doesNotMatch(first.plist, /--no-base-prompt/);
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
    () => renderDisabledLaunchAgent(manifest, identityMap, "viatica", { basePromptPath: "relative.md" }),
    /must be absolute/,
  );
  assert.throws(
    () => renderDisabledLaunchAgent(manifest, identityMap, "nexus", { basePromptPath: "relative.md" }),
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

test("Fleet can parameterize the canonical WSS relay without hardcoding a private host", () => {
  const rendered = renderDisabledLaunchAgent(manifest, identityMap, "nexus", {
    relayUrl: "wss://buzz.example.test",
  });
  assert.equal(
    rendered.argv[rendered.argv.indexOf("--relay-url") + 1],
    "wss://buzz.example.test",
  );
  assert.doesNotMatch(JSON.stringify(manifest), /buzz\.example\.test/);
  assert.throws(
    () => renderDisabledLaunchAgent(manifest, identityMap, "nexus", { relayUrl: "https://buzz.example.test" }),
    /absolute ws/,
  );
});

test("Fleet can preserve the live response policy and benign launch environment", () => {
  const first = "c5f9e0a85da537e107fdcc60ea7ee7e1c5e2b5ac0691a30b6566b3f043a50455";
  const second = "7924cc1dd5389c567ea4ad2b3013b71df28c7b856247365efcccbc7763bfdb7f";
  const rendered = renderDisabledLaunchAgent(manifest, identityMap, "nexus", {
    respondTo: "allowlist",
    allowedRespondTo: "owner-only,allowlist",
    respondToAllowlist: `${first},${second}`,
    additionalEnvironment: { CI: "1", NO_COLOR: "1" },
  });
  assert.equal(rendered.argv[rendered.argv.indexOf("--respond-to") + 1], "allowlist");
  assert.equal(
    rendered.argv[rendered.argv.indexOf("--allowed-respond-to") + 1],
    "owner-only,allowlist",
  );
  assert.equal(
    rendered.argv[rendered.argv.indexOf("--respond-to-allowlist") + 1],
    `${first},${second}`,
  );
  assert.match(rendered.plist, /<key>CI<\/key><string>1<\/string>/);
  assert.match(rendered.plist, /<key>NO_COLOR<\/key><string>1<\/string>/);
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

test("Fleet can install a trusted private-office prompt outside the working directory", () => {
  const rendered = renderDisabledLaunchAgent(manifest, identityMap, "viatica", {
    workingDirectory: "/Users/operator",
    basePromptPath: "/owned/prompts/viatica-private-office.md",
  });
  assert.equal(
    rendered.argv[rendered.argv.indexOf("--base-prompt-file") + 1],
    "/owned/prompts/viatica-private-office.md",
  );
  assert.doesNotMatch(rendered.plist, /--no-base-prompt/);
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

test("semantic health rejects green-but-mute workers", () => {
  const base = {
    aspect: "nexus",
    sessionKey: "agent:main:buzz-private",
    state: "running",
    startup: {
      agentPoolReady: true,
      relayConnected: true,
      privateOfficeSubscribed: true,
    },
  };
  const mute = evaluateSemanticHealth(base);
  assert.equal(mute.healthy, false);
  assert.deepEqual(mute.failures, [
    "request_event_missing",
    "reply_event_missing",
    "gateway_session_mismatch",
    "fresh_run_missing",
    "trusted_reply_tool_mismatch",
    "trusted_reply_tool_count_mismatch",
  ]);

  const healthy = evaluateSemanticHealth({
    ...base,
    receipt: {
      requestEventId: "request-1",
      replyEventId: "reply-1",
      replyTo: "request-1",
      sessionKey: "agent:main:buzz-private",
      runId: "run-1",
      toolName: "buzz_nexus_reply",
      toolCallCount: 1,
    },
  });
  assert.deepEqual(healthy, { healthy: true, failures: [] });
});

test("semantic health command fails closed until a functional reply path is proven", () => {
  const evidence = {
    aspect: "nexus",
    sessionKey: "agent:main:buzz-private",
    state: "running",
    startup: {
      agentPoolReady: true,
      relayConnected: true,
      privateOfficeSubscribed: true,
    },
  };
  const inputPath = join(
    process.env.TMPDIR ?? "/tmp",
    `buzz-semantic-health-${process.pid}-${Date.now()}.json`,
  );
  fs.writeFileSync(inputPath, JSON.stringify(evidence));
  try {
    assert.throws(
      () => execFileSync(process.execPath, [join(here, "semantic-health.mjs"), inputPath]),
      (error) => {
        const result = JSON.parse(error.stdout.toString());
        assert.equal(result.ok, false);
        assert.ok(result.checks[0].failures.includes("request_event_missing"));
        return true;
      },
    );
  } finally {
    fs.rmSync(inputPath, { force: true });
  }
});
