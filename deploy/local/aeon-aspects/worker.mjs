import fs from "node:fs";

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
  if (manifest.posture?.basePrompt !== false) errors.push("Buzz base prompt must be disabled");
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
  if (manifest.supervisor?.runAtLoad !== false || manifest.supervisor?.startOnAppLaunch !== false) {
    errors.push("workers must not start automatically");
  }
  if (manifest.supervisor?.restartOnFailure !== false) {
    errors.push("restartOnFailure must remain false until durable request state exists");
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
  }
  warnings.push("avatar metadata is absent from identity-map.json; live profile avatar validation remains open");
  return { ok: errors.length === 0, errors, warnings };
}

export function renderWorker(manifest, identityMap, aspect, tokenFile = "${AEON_GATEWAY_TOKEN_FILE}") {
  const worker = manifest.workers.find((item) => item.aspect === aspect);
  if (!worker) throw new Error(`unknown Aspect: ${aspect}`);
  const member = identityMap.members[aspect];
  const configPath = `deploy/local/aeon-aspects/config/${aspect}.toml`;
  return {
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
      "--no-memory", "--no-base-prompt", "--dedup", "queue", "--multiple-event-handling", "queue", "--relay-observer", "--trusted-inbound-envelope",
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

export function renderDisabledLaunchAgent(manifest, identityMap, aspect, options = {}) {
  const buzzAcpPath = options.buzzAcpPath ?? "/Volumes/AEON/Projects/buzz/target/release/buzz-acp";
  const openclawPath = options.openclawPath ?? "/REQUIRES_FLEET/immutable-openclaw/bin/openclaw";
  const tokenFile = options.tokenFile ?? "/REQUIRES_FLEET/owned-token-file";
  const workingDirectory = options.workingDirectory ?? "/Volumes/AEON/Projects/buzz";
  for (const [label, value] of Object.entries({ buzzAcpPath, openclawPath, tokenFile, workingDirectory })) {
    if (!value.startsWith("/")) throw new Error(`${label} must be absolute`);
    assertArgSafe(value, label);
  }
  const rendered = renderWorker(manifest, identityMap, aspect, tokenFile);
  const agentCommandIndex = rendered.args.indexOf("--agent-command") + 1;
  rendered.args[agentCommandIndex] = openclawPath;
  const configIndex = rendered.args.indexOf("--config") + 1;
  rendered.args[configIndex] = `${workingDirectory}/${rendered.args[configIndex]}`;
  const argv = [buzzAcpPath, ...rendered.args];
  const worker = manifest.workers.find((item) => item.aspect === aspect);
  const stdout = `/Volumes/AEON/Projects/buzz-data/logs/${aspect}.buzz-acp.log`;
  const stderr = `/Volumes/AEON/Projects/buzz-data/logs/${aspect}.buzz-acp.err.log`;
  const argsXml = argv.map((arg) => `    <string>${xml(arg)}</string>`).join("\n");
  return {
    aspect,
    label: rendered.label,
    enabled: false,
    runAtLoad: false,
    keepAlive: false,
    argv,
    privateKeyFile: identityMap.members[aspect].secret_ref,
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
  <key>WorkingDirectory</key><string>${xml(workingDirectory)}</string>
  <key>RunAtLoad</key><false/>
  <key>KeepAlive</key><false/>
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
