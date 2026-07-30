import fs from "node:fs";

const PRIVATE_OFFICE_PROMPT_PREFIX = "deploy/local/aeon-aspects/prompts";

export function renderPrivateOfficePrompt(template, aspect) {
  if (!/^[a-z][a-z0-9-]*$/.test(aspect)) {
    throw new Error(`invalid Aspect prompt slug: ${aspect}`);
  }
  const rendered = template
    .replaceAll("{{ROOM}}", `#aspect-${aspect}`)
    .replaceAll("{{REPLY_TOOL}}", `buzz_${aspect}_reply`);
  if (rendered.includes("{{")) {
    throw new Error(`${aspect}: unresolved private-office prompt token`);
  }
  return rendered;
}

export function loadJson(path) {
  return JSON.parse(fs.readFileSync(path, "utf8"));
}

export function validateManifest(manifest, identityMap) {
  const errors = [];
  const warnings = [];
  if (manifest.enabled !== false) errors.push("package must be disabled by default");
  if (manifest.workers?.length !== 6) errors.push("exactly six Aspect workers are required");
  if (manifest.buzz?.relayUrl !== "ws://localhost:3000") errors.push("Buzz relay must use localhost");
  if (manifest.posture?.memory !== false) errors.push("Buzz memory injection must be disabled");
  if (manifest.posture?.basePrompt !== false) errors.push("compiled generic Buzz base prompt must be disabled");
  if (manifest.posture?.respondTo !== "owner-only") errors.push("respondTo must be owner-only");
  if (manifest.posture?.agents !== 1) errors.push("each worker must have one ACP subprocess");
  if (manifest.posture?.dedup !== "queue") errors.push("dedup must be queue");
  if (manifest.posture?.multipleEventHandling !== "queue") errors.push("multipleEventHandling must be queue");
  if (manifest.posture?.presence !== true) errors.push("presence must remain enabled");
  if (manifest.posture?.typing !== true) errors.push("typing must remain enabled");
  if (manifest.posture?.relayObserver !== true) errors.push("relay observer must remain enabled for receipts");
  if (manifest.posture?.trustedInboundEnvelope !== true) errors.push("trusted inbound envelope must remain enabled");
  if (manifest.posture?.permissionMode !== "bypass-permissions") errors.push("permission mode must be explicitly bypass-permissions");
  if (manifest.posture?.heartbeatIntervalSecs !== 0) errors.push("ACP heartbeat prompting must be disabled");
  if (manifest.posture?.turnLivenessSecs !== 10) errors.push("turn liveness must be 10 seconds");
  if (manifest.posture?.idleTimeoutSecs !== 900) errors.push("idle timeout must be 900 seconds");
  if (manifest.posture?.maxTurnDurationSecs !== 7200) errors.push("max turn duration must be 7200 seconds");
  if (manifest.posture?.contextMessageLimit !== 12) errors.push("context message limit must be 12");
  if (manifest.posture?.maxTurnsPerSession !== 0) errors.push("Buzz session rotation must be disabled");
  const tokenContract = manifest.gateway?.tokenFileContract;
  if (
    tokenContract?.absolute !== true || tokenContract?.regular !== true ||
    tokenContract?.symlink !== false || tokenContract?.owner !== "current-user" ||
    tokenContract?.mode !== "0600"
  ) {
    errors.push("Gateway token file contract must require absolute regular non-symlink current-user 0600");
  }
  if (manifest.supervisor?.runAtLoad !== true || manifest.supervisor?.startOnAppLaunch !== true) {
    errors.push("workers must retain live supervised startup");
  }
  if (manifest.supervisor?.restartOnFailure !== true) {
    errors.push("workers must retain live restart supervision");
  }
  const concilium = identityMap.channels?.concilium;
  if (concilium?.channel_id !== manifest.buzz?.conciliumChannelId) errors.push("Concilium UUID drift");
  const architect = identityMap.members?.architect;
  if (architect?.pubkey_hex !== manifest.buzz?.architectPubkey) errors.push("Architect pubkey drift");

  for (const worker of manifest.workers ?? []) {
    const member = identityMap.members?.[worker.aspect];
    const channel = identityMap.channels?.[`aspect_${worker.aspect}`];
    if (!member) { errors.push(`${worker.aspect}: missing identity-map member`); continue; }
    if (worker.displayName !== member.display_name) errors.push(`${worker.aspect}: display name drift`);
    if (worker.pubkey !== member.pubkey_hex) errors.push(`${worker.aspect}: pubkey drift`);
    if (worker.gatewayAgentId !== member.gateway_agent_id) errors.push(`${worker.aspect}: Gateway agent drift`);
    if (worker.privateChannelId !== channel?.channel_id) errors.push(`${worker.aspect}: private room drift`);
    const expectedMembers = JSON.stringify(["architect", worker.aspect]);
    if (JSON.stringify(channel?.members) !== expectedMembers) errors.push(`${worker.aspect}: private room membership is not exact`);
    if (concilium?.channel_id === worker.privateChannelId) errors.push(`${worker.aspect}: private room is Concilium`);
    if (worker.sessionKey !== `agent:${worker.gatewayAgentId}:buzz-private`) errors.push(`${worker.aspect}: unstable session key`);
    if (!member.secret_ref) errors.push(`${worker.aspect}: missing private-key reference`);
    const expectedPromptFile = `${PRIVATE_OFFICE_PROMPT_PREFIX}/${worker.aspect}-private-office.md`;
    if (worker.basePromptFile !== expectedPromptFile) {
      errors.push(`${worker.aspect}: trusted private-office base prompt drift`);
    }
  }
  warnings.push("avatar metadata is absent from identity-map.json; live profile avatar validation remains open");
  return { ok: errors.length === 0, errors, warnings };
}

export function renderWorker(manifest, identityMap, aspect, tokenFile = "${AEON_GATEWAY_TOKEN_FILE}") {
  const worker = manifest.workers.find((item) => item.aspect === aspect);
  if (!worker) throw new Error(`unknown Aspect: ${aspect}`);
  const member = identityMap.members[aspect];
  const configPath = `deploy/local/aeon-aspects/config/${aspect}.toml`;
  const basePromptArgs = worker.basePromptFile
    ? ["--base-prompt-file", worker.basePromptFile]
    : ["--no-base-prompt"];
  const rendered = {
    enabled: false,
    label: `org.aeon.buzz-acp.${aspect}`,
    command: "buzz-acp",
    args: [
      "--relay-url", manifest.buzz.relayUrl,
      "--private-key-file", member.secret_ref,
      "--expected-public-key", worker.pubkey,
      "--agent-owner", manifest.buzz.architectPubkey,
      "--agent-command", "openclaw",
      "--agent-args", ["acp", "--session", worker.sessionKey, "--require-existing", "--token-file", tokenFile, "--url", manifest.gateway.url, "--provenance", manifest.gateway.provenance, "--no-prefix-cwd"].join(","),
      "--agents", "1", "--subscribe", "config", "--config", configPath,
      "--respond-to", "owner-only", "--allowed-respond-to", "owner-only",
      "--no-memory", ...basePromptArgs, "--dedup", "queue", "--multiple-event-handling", "queue", "--relay-observer", "--trusted-inbound-envelope", "--no-agent-publisher-credentials",
      "--permission-mode", manifest.posture.permissionMode,
      "--heartbeat-interval", String(manifest.posture.heartbeatIntervalSecs),
      "--turn-liveness-secs", String(manifest.posture.turnLivenessSecs),
      "--idle-timeout", String(manifest.posture.idleTimeoutSecs),
      "--max-turn-duration", String(manifest.posture.maxTurnDurationSecs),
      "--context-message-limit", String(manifest.posture.contextMessageLimit),
      "--max-turns-per-session", String(manifest.posture.maxTurnsPerSession),
      "--turn-receipts", "--expected-gateway-session-key", worker.sessionKey
    ],
    privateKeyRef: member.secret_ref,
    sessionKey: worker.sessionKey,
    supervisor: manifest.supervisor
  };
  assertTrustedPublisherContract(rendered.args, aspect, worker.basePromptFile);
  return rendered;
}

function xml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function assertArgSafe(value, label) {
  if (/[\0\r\n,]/.test(value)) throw new Error(`${label} contains a forbidden delimiter`);
}

function countArg(argv, expected) {
  return argv.filter((value) => value === expected).length;
}

export function assertTrustedPublisherContract(argv, aspect, expectedBasePromptFile) {
  if (!Array.isArray(argv)) throw new Error(`${aspect}: worker argv must be an array`);
  for (const flag of [
    "--no-agent-publisher-credentials",
    "--trusted-inbound-envelope",
    "--base-prompt-file",
    "--turn-receipts",
  ]) {
    const count = countArg(argv, flag);
    if (count !== 1) {
      throw new Error(`${aspect}: trusted publisher contract requires exactly one ${flag}; found ${count}`);
    }
  }
  if (argv.includes("--no-base-prompt")) {
    throw new Error(`${aspect}: trusted publisher contract forbids --no-base-prompt`);
  }
  const basePromptIndex = argv.indexOf("--base-prompt-file");
  if (argv[basePromptIndex + 1] !== expectedBasePromptFile) {
    throw new Error(`${aspect}: trusted publisher contract base prompt drift`);
  }
}

export function evaluateSemanticHealth({ aspect, sessionKey, state, startup, receipt }) {
  const failures = [];
  if (state !== "running") failures.push("worker_not_running");
  if (startup?.agentPoolReady !== true) failures.push("agent_pool_not_ready");
  if (startup?.relayConnected !== true) failures.push("relay_not_connected");
  if (startup?.privateOfficeSubscribed !== true) failures.push("private_office_not_subscribed");
  if (!receipt?.requestEventId) failures.push("request_event_missing");
  if (!receipt?.replyEventId) failures.push("reply_event_missing");
  if (receipt?.replyTo !== receipt?.requestEventId) failures.push("reply_anchor_mismatch");
  if (receipt?.sessionKey !== sessionKey) failures.push("gateway_session_mismatch");
  if (!receipt?.runId) failures.push("fresh_run_missing");
  if (receipt?.toolName !== `buzz_${aspect}_reply`) failures.push("trusted_reply_tool_mismatch");
  if (receipt?.toolCallCount !== 1) failures.push("trusted_reply_tool_count_mismatch");
  return { healthy: failures.length === 0, failures };
}

export function renderDisabledLaunchAgent(manifest, identityMap, aspect, options = {}) {
  const buzzAcpPath = options.buzzAcpPath ?? "/Volumes/AEON/Projects/buzz/target/release/buzz-acp";
  const openclawPath = options.openclawPath ?? "/REQUIRES_FLEET/immutable-openclaw/bin/openclaw";
  const tokenFile = options.tokenFile ?? "/REQUIRES_FLEET/owned-token-file";
  const privateKeyFile = options.privateKeyFile ?? identityMap.members[aspect].secret_ref;
  const workingDirectory = options.workingDirectory ?? "/Volumes/AEON/Projects/buzz";
  const configPath = options.configPath ?? null;
  const basePromptPath = options.basePromptPath ?? null;
  const stdoutPath = options.stdoutPath ?? null;
  const stderrPath = options.stderrPath ?? null;
  const launcherPath = options.launcherPath ?? null;
  const executablePath = options.executablePath ?? null;
  const openclawConfigPath = options.openclawConfigPath ?? null;
  const openclawStateDir = options.openclawStateDir ?? null;
  const relayUrl = options.relayUrl ?? manifest.buzz.relayUrl;
  const respondTo = options.respondTo ?? null;
  const allowedRespondTo = options.allowedRespondTo ?? null;
  const respondToAllowlist = options.respondToAllowlist ?? null;
  const additionalEnvironment = options.additionalEnvironment ?? {};
  const agentCommandPrefixArgs = options.agentCommandPrefixArgs ?? [];
  if (!/^wss?:\/\/[^,\s]+$/.test(relayUrl)) {
    throw new Error("relayUrl must be an absolute ws:// or wss:// URL");
  }
  assertArgSafe(relayUrl, "relayUrl");
  if ((respondTo === null) !== (allowedRespondTo === null)) {
    throw new Error("respondTo and allowedRespondTo must be supplied together");
  }
  for (const [label, value] of Object.entries({
    ...(respondTo !== null ? { respondTo } : {}),
    ...(allowedRespondTo !== null ? { allowedRespondTo } : {}),
  })) {
    if (!/^[a-z-]+(?:,[a-z-]+)*$/.test(value)) {
      throw new Error(`${label} contains an invalid response mode`);
    }
  }
  if (
    respondToAllowlist !== null &&
    !/^[0-9a-f]{64}(?:,[0-9a-f]{64})*$/.test(respondToAllowlist)
  ) {
    throw new Error("respondToAllowlist must be a comma-separated lowercase pubkey list");
  }
  if (
    additionalEnvironment === null ||
    Array.isArray(additionalEnvironment) ||
    typeof additionalEnvironment !== "object"
  ) {
    throw new Error("additionalEnvironment must be an object");
  }
  for (const [key, value] of Object.entries(additionalEnvironment)) {
    if (!/^[A-Z][A-Z0-9_]*$/.test(key) || typeof value !== "string" || /[\0\r\n]/.test(value)) {
      throw new Error(`additionalEnvironment.${key} is invalid`);
    }
  }
  if (!Array.isArray(agentCommandPrefixArgs)) {
    throw new Error("agentCommandPrefixArgs must be an array");
  }
  if (agentCommandPrefixArgs.length > 1) {
    throw new Error("agentCommandPrefixArgs accepts exactly one OpenClaw entrypoint");
  }
  for (const [index, value] of agentCommandPrefixArgs.entries()) {
    if (typeof value !== "string" || !value.startsWith("/") || !value.endsWith("/openclaw.mjs")) {
      throw new Error(`agentCommandPrefixArgs[${index}] must be absolute`);
    }
    assertArgSafe(value, `agentCommandPrefixArgs[${index}]`);
  }
  for (const [label, value] of Object.entries({
    buzzAcpPath,
    openclawPath,
    tokenFile,
    privateKeyFile,
    workingDirectory,
    ...(configPath !== null ? { configPath } : {}),
    ...(basePromptPath !== null ? { basePromptPath } : {}),
    ...(stdoutPath !== null ? { stdoutPath } : {}),
    ...(stderrPath !== null ? { stderrPath } : {}),
    ...(launcherPath !== null ? { launcherPath } : {}),
    ...(openclawConfigPath !== null ? { openclawConfigPath } : {}),
    ...(openclawStateDir !== null ? { openclawStateDir } : {}),
  })) {
    if (!value.startsWith("/")) throw new Error(`${label} must be absolute`);
    assertArgSafe(value, label);
  }
  if ((openclawConfigPath === null) !== (openclawStateDir === null)) {
    throw new Error("openclawConfigPath and openclawStateDir must be supplied together");
  }
  if (executablePath !== null) {
    if (!executablePath.split(":").every((entry) => entry.startsWith("/"))) {
      throw new Error("executablePath entries must be absolute");
    }
    assertArgSafe(executablePath, "executablePath");
  }
  const launchIdentityMap = {
    ...identityMap,
    members: {
      ...identityMap.members,
      [aspect]: { ...identityMap.members[aspect], secret_ref: privateKeyFile },
    },
  };
  const rendered = renderWorker(manifest, launchIdentityMap, aspect, tokenFile);
  const relayUrlIndex = rendered.args.indexOf("--relay-url") + 1;
  rendered.args[relayUrlIndex] = relayUrl;
  if (respondTo !== null) {
    rendered.args[rendered.args.indexOf("--respond-to") + 1] = respondTo;
    rendered.args[rendered.args.indexOf("--allowed-respond-to") + 1] = allowedRespondTo;
    const existingAllowlist = rendered.args.indexOf("--respond-to-allowlist");
    if (existingAllowlist >= 0) rendered.args.splice(existingAllowlist, 2);
    if (respondToAllowlist !== null) {
      const insertAt = rendered.args.indexOf("--allowed-respond-to") + 2;
      rendered.args.splice(insertAt, 0, "--respond-to-allowlist", respondToAllowlist);
    }
  } else if (respondToAllowlist !== null) {
    throw new Error("respondToAllowlist requires respondTo and allowedRespondTo");
  }
  const agentCommandIndex = rendered.args.indexOf("--agent-command") + 1;
  rendered.args[agentCommandIndex] = openclawPath;
  if (agentCommandPrefixArgs.length > 0) {
    const agentArgsIndex = rendered.args.indexOf("--agent-args") + 1;
    rendered.args[agentArgsIndex] = [agentCommandPrefixArgs.join(","), rendered.args[agentArgsIndex]].join(",");
  }
  const configIndex = rendered.args.indexOf("--config") + 1;
  rendered.args[configIndex] = configPath ?? `${workingDirectory}/${rendered.args[configIndex]}`;
  const basePromptIndex = rendered.args.indexOf("--base-prompt-file") + 1;
  if (basePromptIndex > 0) {
    rendered.args[basePromptIndex] =
      basePromptPath ?? `${workingDirectory}/${rendered.args[basePromptIndex]}`;
  } else if (basePromptPath !== null) {
    throw new Error(`${aspect}: basePromptPath supplied for worker without a custom base prompt`);
  }
  assertTrustedPublisherContract(
    rendered.args,
    aspect,
    rendered.args[rendered.args.indexOf("--base-prompt-file") + 1],
  );
  // launchd may reject direct execution of provenance-marked development binaries.
  // A Fleet-owned system launcher keeps the binary and its digest explicit in argv.
  const argv = [...(launcherPath ? [launcherPath] : []), buzzAcpPath, ...rendered.args];
  const worker = manifest.workers.find((item) => item.aspect === aspect);
  const stdout = stdoutPath ?? `/Volumes/AEON/Projects/buzz-data/logs/${aspect}.buzz-acp.log`;
  const stderr = stderrPath ?? `/Volumes/AEON/Projects/buzz-data/logs/${aspect}.buzz-acp.err.log`;
  const argsXml = argv.map((arg) => `    <string>${xml(arg)}</string>`).join("\n");
  const environment = {
    ...additionalEnvironment,
    ...(executablePath ? { PATH: executablePath } : {}),
    ...(openclawConfigPath ? { OPENCLAW_CONFIG_PATH: openclawConfigPath } : {}),
    ...(openclawStateDir ? { OPENCLAW_STATE_DIR: openclawStateDir } : {}),
  };
  const environmentEntries = Object.entries(environment)
    .map(([key, value]) => `<key>${key}</key><string>${xml(value)}</string>`)
    .join("");
  const environmentXml = environmentEntries
    ? `\n  <key>EnvironmentVariables</key>\n  <dict>${environmentEntries}</dict>`
    : "";
  return {
    aspect,
    label: rendered.label,
    enabled: false,
    runAtLoad: manifest.supervisor.runAtLoad,
    keepAlive: manifest.supervisor.restartOnFailure,
    argv,
    privateKeyFile,
    tokenFile,
    tokenFileContract: manifest.gateway.tokenFileContract,
    expectedPublicKey: worker.pubkey,
    rollback: ["launchctl", "bootout", `gui/<uid>/${rendered.label}`],
    plist: `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>${xml(rendered.label)}</string>
  <key>ProgramArguments</key>
  <array>
${argsXml}
  </array>
  <key>WorkingDirectory</key><string>${xml(workingDirectory)}</string>${environmentXml}
  <key>RunAtLoad</key><${manifest.supervisor.runAtLoad}/>
  <key>KeepAlive</key><${manifest.supervisor.restartOnFailure}/>
  <key>ProcessType</key><string>Background</string>
  <key>StandardOutPath</key><string>${xml(stdout)}</string>
  <key>StandardErrorPath</key><string>${xml(stderr)}</string>
</dict>
</plist>
`,
  };
}

export function correlateReceipt({ triggeringEventIds, replyEvents, sessionKey, runId }) {
  if (!Array.isArray(triggeringEventIds) || triggeringEventIds.length !== 1) throw new Error("receipt requires exactly one request event");
  const requestEventId = triggeringEventIds[0];
  const matches = replyEvents.filter((event) => event.replyTo === requestEventId);
  if (matches.length !== 1) throw new Error(`receipt requires exactly one anchored reply; found ${matches.length}`);
  if (!sessionKey || !runId) throw new Error("receipt requires Gateway session key and run id");
  return { requestEventId, replyEventId: matches[0].eventId, gatewaySessionKey: sessionKey, runId };
}
