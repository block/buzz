import { spawnSync } from "node:child_process";
import fs from "node:fs";
import { createHash } from "node:crypto";
import path from "node:path";

const HEX_64 = /^[0-9a-f]{64}$/;
const SAFE_LABEL = /^[a-z0-9][a-z0-9._-]*$/;
const REQUIRED_AGENT_MODE = "agent-full-access";
const REQUIRED_CODEX_ACP_VERSION = "1.1.7";
const REQUIRED_CLAUDE_ACP_VERSION = "0.62.0";
const REQUIRED_CLAUDE_CODE_VERSION = "2.1.220";
const REQUIRED_CURSOR_CLI_VERSION = "2026.07.23-e383d2b";
const REQUIRED_CLAUDE_ACP_INTEGRITY =
  "sha512-8QRNmyk5Cfy4XVREeg5KCPoCDtmYS0xALY9WqI640PfopLMpeUzMByXbzLkBLbD819zB67DBhLG5ta98uOEPKg==";
const REQUIRED_CLAUDE_ACP_GIT_HEAD = "53a0c36ce3b0b76929d11d8b9565e319da745608";
const REQUIRED_CLAUDE_ACP_ENTRYPOINT_SHA256 =
  "260aac90bf75f197b93640087c1de66441761d43c2784efa035fdcee60b5dacd";
const REQUIRED_CLAUDE_ACP_CLOSURE_SHA256 =
  "ba5650a750d25811f36f4e6e91ad079d700743ddfb4f52abb90d46c9e9d86002";
const REQUIRED_CLAUDE_CODE_SHA256 =
  "8addc857f3fe64d5a0368af9ee50321b50afb4a6918ba3ef018ab84f5dbbe081";
const REQUIRED_CLAUDE_RUNTIME_ROOT = "/Users/architect/Library/Application Support/AEON/aeon-v6";
const REQUIRED_SHARED_BUZZ_ACP_BINARY = `${REQUIRED_CLAUDE_RUNTIME_ROOT}/bin/buzz-acp`;
const REQUIRED_SHARED_BUZZ_ACP_SHA256 =
  "1d260060a0b790645a0455d23c7a82ac7836193108673a76f44423c5d81be9be";
const REQUIRED_CURSOR_BOOTSTRAP_PATH = `${REQUIRED_CLAUDE_RUNTIME_ROOT}/buzz/cursor-acp-bootstrap.cjs`;
const REQUIRED_CURSOR_BOOTSTRAP_SHA256 =
  "b3f4e90e675bd0e8f0827b618203c33b9904cbd01becd2b80fc868d75b8797e8";
const REQUIRED_CLAUDE_NODE = {
  version: "v24.1.0",
  sha256: "59450bb6448c8a40b3f3b86da45c3babb2e0503e04c47e5a715e8e137389878b",
  mode: "0755",
  binary: "/Users/architect/.nvm/versions/node/v24.1.0/bin/node",
};
const REQUIRED_CLAUDE_AUTH = {
  mode: "existing-claude-subscription",
  authMethod: "claude.ai",
  provider: "firstParty",
  subscriptionTypes: ["pro"],
};
const ANTHROPIC_CREDENTIAL_ENV = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"];
const CURSOR_OVERRIDE_ENV = ["CURSOR_API_KEY", "CURSOR_API_ENDPOINT"];
const REQUIRED_SHARED_ROOMS = ["ops", "concilium"];
const REQUIRED_OFFICE_ROOMS = [
  "aspect_nexus",
  "aspect_mechanon",
  "aspect_fontis",
  "aspect_sapientis",
  "aspect_viatica",
  "aspect_voxis",
];
const REQUIRED_INBOUND_MEMBERS = [
  "architect",
  "nexus",
  "mechanon",
  "fontis",
  "sapientis",
  "viatica",
  "voxis",
];
export const REQUIRED_ROOM_NAMES = [...REQUIRED_SHARED_ROOMS, ...REQUIRED_OFFICE_ROOMS];
const REQUIRED_CURSOR_ROOT = `/Users/architect/.local/share/cursor-agent/versions/${REQUIRED_CURSOR_CLI_VERSION}`;
const REQUIRED_CURSOR_CLI = {
  package: "cursor-agent",
  version: REQUIRED_CURSOR_CLI_VERSION,
  binary: "/Users/architect/.local/bin/cursor-agent",
  root: REQUIRED_CURSOR_ROOT,
  entrypointSha256: "eed61c5224668c9236334c4c68936a16aecc37374b592f59e31eb50433817831",
  closureSha256: "400227a16df5e9f7bb4273f176cf68e41ef499f06fac5e6c9c6c3556ab2cc726",
  args: ["--trust", "acp"],
  auth: {
    mode: "existing-cursor-subscription",
    status: "authenticated",
    subscriptionTypes: ["Pro"],
  },
  model: {
    requested: "cursor-grok-4.5-high",
    effective: "grok-4.5[effort=high,fast=true]",
    selectionStatus: "upstream_limited_to_fast_wire_variant",
  },
};
const REQUIRED_GROK_CLI = {
  package: "grok-build",
  version: "0.2.93",
  build: "f00f96316d4b",
  binary: "/Users/architect/.grok/bin/grok",
  realBinary: "/Users/architect/.grok/downloads/grok-0.2.93-macos-aarch64",
  entrypointSha256: "2a97ba675bd992aa9b981e2e83776460d94f469b510c0b8efe28b50d236d767c",
  args: ["agent", "--model", "grok-4.5", "--reasoning-effort", "high", "--always-approve", "stdio"],
  auth: {
    mode: "existing-grok-login",
    provider: "grok.com",
    authFile: "/Users/architect/.grok/auth.json",
  },
  model: {
    requested: "grok-4.5-high",
    effective: "grok-4.5",
    reasoningEffort: "high",
  },
};
const GROK_OVERRIDE_ENV = [
  "XAI_API_KEY",
  "GROK_CODE_XAI_API_KEY",
  "GROK_AUTH",
  "GROK_AUTH_PATH",
  "GROK_HOME",
  "GROK_AUTH_PROVIDER_COMMAND",
  "GROK_OIDC_ISSUER",
  "GROK_OIDC_CLIENT_ID",
  "GROK_CLI_CHAT_PROXY_BASE_URL",
  "XAI_API_BASE_URL",
];
const ENV_BINARY = "/usr/bin/env";
const WORKER_CONTRACTS = {
  codex_cli: {
    principal: "codex_cli",
    adapterKey: "codexAcp",
    adapterPackage: "@agentclientprotocol/codex-acp",
    adapterVersion: REQUIRED_CODEX_ACP_VERSION,
    label: "org.aeon.buzz-acp.codex-cli",
  },
  claude_cli: {
    principal: "claude_code",
    adapterKey: "claudeAcp",
    adapterPackage: "@agentclientprotocol/claude-agent-acp",
    adapterVersion: REQUIRED_CLAUDE_ACP_VERSION,
    label: "org.aeon.buzz-acp.claude-cli",
  },
  cursor_cli: {
    principal: "cursor_cli",
    adapterKey: "cursorAcp",
    adapterPackage: "cursor-agent",
    adapterVersion: REQUIRED_CURSOR_CLI_VERSION,
    label: "org.aeon.buzz-acp.cursor-cli",
    native: true,
  },
  grok_cli: {
    principal: "grok_cli",
    adapterKey: "grokAcp",
    adapterPackage: "grok-build",
    adapterVersion: REQUIRED_GROK_CLI.version,
    label: "org.aeon.buzz-acp.grok-cli",
    native: true,
  },
};

export function loadJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function isAbsoluteSafePath(value) {
  return typeof value === "string" && path.isAbsolute(value) && !/[\0\r\n,]/.test(value);
}

export function hashPackageClosure(root, ignoredTopLevel = []) {
  if (!isAbsoluteSafePath(root)) throw new Error("package root must be an absolute safe path");
  const hash = createHash("sha256");
  const entries = [];

  function visit(directory, relativeDirectory) {
    for (const name of fs.readdirSync(directory).sort()) {
      if (!relativeDirectory && ignoredTopLevel.includes(name)) continue;
      const absolutePath = path.join(directory, name);
      const relativePath = relativeDirectory ? `${relativeDirectory}/${name}` : name;
      const stat = fs.lstatSync(absolutePath);
      if (stat.isDirectory()) {
        visit(absolutePath, relativePath);
      } else if (stat.isFile()) {
        entries.push({
          kind: "file",
          absolutePath,
          relativePath,
          size: stat.size,
        });
      } else if (stat.isSymbolicLink()) {
        entries.push({
          kind: "symlink",
          relativePath,
          target: fs.readlinkSync(absolutePath),
        });
      } else {
        throw new Error(`unsupported package entry: ${relativePath}`);
      }
    }
  }

  visit(root, "");
  entries.sort((left, right) => {
    if (left.relativePath === right.relativePath) return 0;
    return left.relativePath < right.relativePath ? -1 : 1;
  });
  for (const entry of entries) {
    if (entry.kind === "symlink") {
      hash.update(`l\0${entry.relativePath}\0${entry.target}\0`);
    } else {
      hash.update(`f\0${entry.relativePath}\0${entry.size}\0`);
      hash.update(fs.readFileSync(entry.absolutePath));
      hash.update("\0");
    }
  }
  return hash.digest("hex");
}

export function hashCursorClosure(root) {
  return hashPackageClosure(root, [".running"]);
}

export function validateAmbientAnthropicCredentials(environment) {
  const present = ANTHROPIC_CREDENTIAL_ENV.filter(
    (name) => typeof environment?.[name] === "string" && environment[name].length > 0,
  );
  return {
    ok: present.length === 0,
    errors: present.map((name) => `${name} must be absent for Claude subscription authentication`),
  };
}

export function validateClaudeSubscriptionAuth(status, contract) {
  const errors = [];
  if (status?.loggedIn !== true) errors.push("Claude Code existing login is unavailable");
  if (status?.authMethod !== contract?.authMethod) {
    errors.push(`Claude Code auth method must be ${contract?.authMethod}`);
  }
  if (status?.apiProvider !== contract?.provider) {
    errors.push(`Claude Code API provider must be ${contract?.provider}`);
  }
  if (!contract?.subscriptionTypes?.includes(status?.subscriptionType)) {
    errors.push(
      `Claude Code subscription type must be one of: ${(contract?.subscriptionTypes ?? []).join(", ")}`,
    );
  }
  return { ok: errors.length === 0, errors };
}

export function validateCursorSubscriptionAuth(status, about, contract) {
  const errors = [];
  if (status?.status !== contract?.status || status?.isAuthenticated !== true) {
    errors.push("Cursor existing subscription login is unavailable");
  }
  if (!contract?.subscriptionTypes?.includes(about?.subscriptionTier)) {
    errors.push(
      `Cursor subscription type must be one of: ${(contract?.subscriptionTypes ?? []).join(", ")}`,
    );
  }
  return { ok: errors.length === 0, errors };
}

export function validateAmbientCursorOverrides(environment) {
  const present = CURSOR_OVERRIDE_ENV.filter(
    (name) => typeof environment?.[name] === "string" && environment[name].length > 0,
  );
  return {
    ok: present.length === 0,
    errors: present.map((name) => `${name} must be absent for Cursor subscription authentication`),
  };
}

export function validateAmbientGrokOverrides(environment) {
  const present = GROK_OVERRIDE_ENV.filter(
    (name) => typeof environment?.[name] === "string" && environment[name].length > 0,
  );
  return {
    ok: present.length === 0,
    errors: present.map((name) => `${name} must be absent for Grok subscription authentication`),
  };
}

export function validatePinnedNodeRuntime(node, environment) {
  let stat;
  try {
    stat = fs.lstatSync(node?.binary);
  } catch {
    return { ok: false, errors: ["pinned Node runtime is missing"] };
  }
  if (!stat.isFile() || stat.isSymbolicLink()) {
    return {
      ok: false,
      errors: ["pinned Node runtime must be a regular non-symlink file"],
    };
  }
  if ((stat.mode & 0o777).toString(8).padStart(4, "0") !== node.mode) {
    return {
      ok: false,
      errors: [`pinned Node runtime mode must be ${node.mode}`],
    };
  }
  try {
    fs.accessSync(node.binary, fs.constants.X_OK);
  } catch {
    return { ok: false, errors: ["pinned Node runtime must be executable"] };
  }
  const sha256 = createHash("sha256").update(fs.readFileSync(node.binary)).digest("hex");
  if (sha256 !== node.sha256) {
    return {
      ok: false,
      errors: ["pinned Node runtime SHA-256 does not match the manifest pin"],
    };
  }
  const version = spawnSync(node.binary, ["--version"], {
    encoding: "utf8",
    env: environment,
  });
  if (version.status !== 0 || version.stdout.trim() !== node.version) {
    return {
      ok: false,
      errors: [`pinned Node runtime version does not match ${node.version}`],
    };
  }
  return { ok: true, errors: [] };
}

function memberPubkey(identityMap, memberId) {
  return identityMap.members?.[memberId]?.pubkey_hex;
}

function workerSelector(manifest) {
  return manifest.worker?.selector ?? manifest.worker?.principal;
}

export function exactRoomIds(manifest, identityMap) {
  return [...manifest.buzz.sharedRooms, ...manifest.buzz.officeRooms].map(
    (roomName) => identityMap.channels?.[roomName]?.channel_id,
  );
}

export function validateSubscriptionProjection(configText, manifest, identityMap) {
  const errors = [];
  const channelArrays = [];
  const ruleNames = [];
  const channelArrayPattern = /^\s*channels\s*=\s*(\[[\s\S]*?^\s*\])/gm;
  const tableHeaderPattern = /^\s*\[[^\r\n]*$/gm;
  const tableHeaders = [...configText.matchAll(tableHeaderPattern)];
  const preamble = configText.slice(0, tableHeaders[0]?.index ?? configText.length);
  const preambleLines = preamble
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#"));
  const validRuleHeader = (header) => /^\s*\[\[rules\]\]\s*(?:#.*)?$/.test(header[0]);
  const ruleSections = tableHeaders.flatMap((header, index) => {
    if (!validRuleHeader(header)) return [];
    const bodyStart = header.index + header[0].length;
    const bodyEnd = tableHeaders[index + 1]?.index ?? configText.length;
    return [configText.slice(bodyStart, bodyEnd)];
  });

  if (/'''|"""/.test(configText)) {
    errors.push("subscription projection must not contain multiline strings");
  }
  if (
    preambleLines.length !== 0 ||
    tableHeaders.length !== 2 ||
    tableHeaders.some((header) => !validRuleHeader(header))
  ) {
    errors.push("subscription projection must not contain content outside the two canonical rules");
  }
  if (ruleSections.length !== 2) {
    errors.push("subscription projection must contain exactly two rules");
  }

  for (const rule of ruleSections) {
    const channelMatches = [...rule.matchAll(channelArrayPattern)];
    if (channelMatches.length !== 1) {
      errors.push("each subscription rule must contain exactly one channel array");
      continue;
    }
    try {
      const channels = JSON.parse(channelMatches[0][1].replace(/,\s*\]$/, "]"));
      if (!Array.isArray(channels) || !channels.every((id) => typeof id === "string")) {
        errors.push("subscription channels must be string arrays");
      } else {
        channelArrays.push(channels);
      }
    } catch {
      errors.push("subscription channels must use deterministic string-array syntax");
    }

    const mentionMatches = [...rule.matchAll(/^\s*require_mention\s*=\s*(true|false)\s*$/gm)];
    if (mentionMatches.length !== 1 || mentionMatches[0][1] !== "true") {
      errors.push("each subscription rule must require a mention");
    }

    const remainingLines = rule
      .replace(channelArrayPattern, "")
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line && !line.startsWith("#"));
    const nameCount = remainingLines.filter((line) =>
      /^name\s*=\s*"[a-z0-9-]+"$/.test(line),
    ).length;
    const kindsCount = remainingLines.filter((line) =>
      /^kinds\s*=\s*\[\s*9\s*,\s*40002\s*\]$/.test(line),
    ).length;
    const mentionCount = remainingLines.filter((line) =>
      /^require_mention\s*=\s*true$/.test(line),
    ).length;
    if (nameCount !== 1 || kindsCount !== 1 || mentionCount !== 1 || remainingLines.length !== 3) {
      errors.push(
        "each subscription rule must use the deterministic name, channels, kinds, and mention schema",
      );
    } else {
      ruleNames.push(
        remainingLines
          .find((line) => line.startsWith("name"))
          .match(/^name\s*=\s*"([a-z0-9-]+)"$/)[1],
      );
    }
  }

  if (
    JSON.stringify(ruleNames) !== JSON.stringify(["aeon-shared-control", "aeon-aspect-offices"])
  ) {
    errors.push("subscription rules must be the canonical shared-control and Aspect-office rules");
  }

  const expectedChannelArrays = [
    manifest.buzz.sharedRooms.map((roomName) => identityMap.channels?.[roomName]?.channel_id),
    manifest.buzz.officeRooms.map((roomName) => identityMap.channels?.[roomName]?.channel_id),
  ];
  const expectedRoomIds = expectedChannelArrays.flat();
  const actualRoomIds = channelArrays.flat();
  if (
    expectedRoomIds.some((id) => typeof id !== "string") ||
    JSON.stringify(channelArrays) !== JSON.stringify(expectedChannelArrays)
  ) {
    errors.push("subscription projection must contain exactly the eight canonical rooms");
  }

  return {
    ok: errors.length === 0,
    errors,
    roomIds: actualRoomIds,
  };
}

export function validateManifest(manifest, identityMap) {
  const errors = [];
  const principal = manifest.worker?.principal;
  const selector = workerSelector(manifest);
  const member = identityMap.members?.[principal];
  const contract = WORKER_CONTRACTS[selector];

  if (manifest.schema !== "aeon_buzz_external_cli_worker_v1") {
    errors.push("unsupported external CLI worker schema");
  }
  if (manifest.enabled !== false) errors.push("external CLI worker must be disabled by default");
  if (!contract) {
    errors.push("worker selector must be codex_cli, claude_cli, cursor_cli, or grok_cli");
  }
  if (contract && principal !== contract.principal) {
    errors.push(`${selector} worker must bind to ${contract.principal}`);
  }
  if (manifest.worker?.agents !== 1) errors.push("exactly one ACP subprocess is required");
  if (!SAFE_LABEL.test(manifest.worker?.label ?? "")) errors.push("invalid launchd label");
  if (contract && manifest.worker?.label !== contract.label) {
    errors.push(`${principal} launchd label drift`);
  }
  if (!member) errors.push(`identity map is missing ${principal}`);
  if (member?.gateway_agent_id !== null || member?.aspect_slug !== null) {
    errors.push(`${principal} must remain an external non-Aspect principal`);
  }
  if (member?.concilium_seat !== principal) errors.push(`${principal} Concilium seat drift`);
  if (!HEX_64.test(member?.pubkey_hex ?? ""))
    errors.push(`${principal} pubkey must be 64 lowercase hex`);
  if (!isAbsoluteSafePath(member?.secret_ref)) {
    errors.push(`${principal} secret_ref must be an absolute safe path`);
  }

  const inbound = manifest.buzz?.allowedInbound ?? [];
  if (JSON.stringify(inbound) !== JSON.stringify(REQUIRED_INBOUND_MEMBERS)) {
    errors.push("inbound allowlist must be exactly Architect and the six canonical Aspects");
  }
  for (const memberId of inbound) {
    if (!HEX_64.test(memberPubkey(identityMap, memberId) ?? "")) {
      errors.push(`${memberId}: inbound identity is missing a valid pubkey`);
    }
  }
  const authorityPrincipals = [
    ...REQUIRED_INBOUND_MEMBERS,
    ...Object.values(WORKER_CONTRACTS).map((workerContract) => workerContract.principal),
  ];
  const authorityPubkeys = authorityPrincipals.map((memberId) =>
    memberPubkey(identityMap, memberId),
  );
  if (new Set(authorityPubkeys).size !== authorityPubkeys.length) {
    errors.push("Architect, Aspect, and external CLI pubkeys must be unique");
  }
  for (const aspectId of REQUIRED_INBOUND_MEMBERS.filter(
    (memberId) => memberId !== "architect",
  )) {
    const officeName = `aspect_${aspectId}`;
    const officeMembers = identityMap.channels?.[officeName]?.members ?? [];
    if (!officeMembers.includes(aspectId)) {
      errors.push(`${officeName}: ${aspectId} is not a member`);
    }
    const conciliumMembers = identityMap.channels?.concilium?.members ?? [];
    if (!conciliumMembers.includes(aspectId)) {
      errors.push(`concilium: ${aspectId} is not a member`);
    }
  }
  if (manifest.buzz?.owner !== "architect") errors.push("Architect must own the worker");
  if (manifest.buzz?.relayUrl !== "ws://localhost:3000") errors.push("relay must remain loopback");
  if (JSON.stringify(manifest.buzz?.sharedRooms) !== JSON.stringify(REQUIRED_SHARED_ROOMS)) {
    errors.push("shared rooms must be exactly ops and concilium");
  }
  if (JSON.stringify(manifest.buzz?.officeRooms) !== JSON.stringify(REQUIRED_OFFICE_ROOMS)) {
    errors.push("office rooms must be exactly the six canonical Aspect offices");
  }
  const roomIds = exactRoomIds(manifest, identityMap);
  if (roomIds.some((roomId) => typeof roomId !== "string"))
    errors.push("configured room is absent from identity map");
  if (new Set(roomIds).size !== roomIds.length) errors.push("configured rooms must be unique");
  for (const roomName of [
    ...(manifest.buzz?.sharedRooms ?? []),
    ...(manifest.buzz?.officeRooms ?? []),
  ]) {
    const members = identityMap.channels?.[roomName]?.members ?? [];
    if (!members.includes(principal)) errors.push(`${roomName}: ${principal} is not a member`);
  }

  const runtime = manifest.runtime;
  const adapter = contract ? runtime?.[contract.adapterKey] : undefined;
  if (contract && adapter?.package !== contract.adapterPackage) {
    errors.push(`${principal} ACP package owner drift`);
  }
  if (contract && adapter?.version !== contract.adapterVersion) {
    errors.push(`${principal} ACP adapter must be pinned to ${contract.adapterVersion}`);
  }
  if (!contract?.native && !/^sha512-[A-Za-z0-9+/]+=*$/.test(adapter?.integrity ?? "")) {
    errors.push(`${principal} ACP integrity must be pinned`);
  }
  if (!HEX_64.test(adapter?.entrypointSha256 ?? "")) {
    errors.push(`${principal} ACP entrypoint SHA-256 must be pinned`);
  }
  if (runtime?.buzzAcpBinary !== REQUIRED_SHARED_BUZZ_ACP_BINARY) {
    errors.push("shared buzz-acp must use the canonical Data-volume path");
  }
  if (runtime?.buzzAcpSha256 !== REQUIRED_SHARED_BUZZ_ACP_SHA256) {
    errors.push("shared buzz-acp checkpoint drift");
  }
  if (selector === "claude_cli") {
    if (adapter?.integrity !== REQUIRED_CLAUDE_ACP_INTEGRITY) {
      errors.push("Claude ACP package integrity drift");
    }
    if (adapter?.gitHead !== REQUIRED_CLAUDE_ACP_GIT_HEAD) {
      errors.push("Claude ACP source checkpoint drift");
    }
    if (adapter?.entrypointSha256 !== REQUIRED_CLAUDE_ACP_ENTRYPOINT_SHA256) {
      errors.push("Claude ACP entrypoint checkpoint drift");
    }
    if (adapter?.closureSha256 !== REQUIRED_CLAUDE_ACP_CLOSURE_SHA256) {
      errors.push("Claude ACP package closure checkpoint drift");
    }
  }
  if (selector === "cursor_cli") {
    if (JSON.stringify(adapter) !== JSON.stringify(REQUIRED_CURSOR_CLI)) {
      errors.push("Cursor CLI runtime, auth, or model contract drift");
    }
  }
  if (selector === "grok_cli") {
    if (JSON.stringify(adapter) !== JSON.stringify(REQUIRED_GROK_CLI)) {
      errors.push("Grok CLI runtime, auth, or model contract drift");
    }
  }
  const usesSafeSupervisor = true;
  for (const [label, value] of Object.entries({
    buzzAcpBinary: runtime?.buzzAcpBinary,
    configPath: runtime?.configPath,
    ...(usesSafeSupervisor
      ? {
          logDir: runtime?.logDir,
          signerPath: runtime?.signerPath,
          supervisorWorkingDirectory: runtime?.supervisorWorkingDirectory,
        }
      : {}),
    ...(selector === "claude_cli" ? { adapterRoot: adapter?.root } : {}),
    ...(selector === "cursor_cli" ? { bootstrapPath: runtime?.bootstrapPath } : {}),
    adapterBinary: adapter?.binary,
  })) {
    if (!isAbsoluteSafePath(value)) errors.push(`${label} must be an absolute safe path`);
  }
  if (selector === "codex_cli") {
    const expectedPaths = {
      configPath: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/buzz/codex-cli.toml`,
      logDir: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/logs`,
      signerPath: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/secrets/codex-cli.sk`,
      systemPromptPath: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/buzz/codex-cli-system.md`,
      supervisorWorkingDirectory: REQUIRED_CLAUDE_RUNTIME_ROOT,
      adapterBinary: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/codex-acp/${REQUIRED_CODEX_ACP_VERSION}/node_modules/.bin/codex-acp`,
    };
    const actualPaths = {
      configPath: runtime?.configPath,
      logDir: runtime?.logDir,
      signerPath: runtime?.signerPath,
      systemPromptPath: runtime?.systemPromptPath,
      supervisorWorkingDirectory: runtime?.supervisorWorkingDirectory,
      adapterBinary: adapter?.binary,
    };
    for (const [label, expected] of Object.entries(expectedPaths)) {
      if (actualPaths[label] !== expected) {
        errors.push(`Codex ${label} must use the launchd-safe Data-volume path`);
      }
    }
    if (!isAbsoluteSafePath(runtime?.codexHome))
      errors.push("codexHome must be an absolute safe path");
    if (runtime?.initialAgentMode !== REQUIRED_AGENT_MODE) {
      errors.push(`INITIAL_AGENT_MODE must be ${REQUIRED_AGENT_MODE}`);
    }
    if (adapter?.model !== "gpt-5.6-sol[medium]") {
      errors.push("Codex ACP model must be gpt-5.6-sol[medium]");
    }
  }
  if (selector === "claude_cli") {
    const claudeCode = runtime?.claudeCode;
    const expectedPaths = {
      configPath: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/buzz/claude-cli.toml`,
      logDir: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/logs`,
      signerPath: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/secrets/claude-code.sk`,
      supervisorWorkingDirectory: REQUIRED_CLAUDE_RUNTIME_ROOT,
      adapterRoot: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/claude-acp/${REQUIRED_CLAUDE_ACP_VERSION}`,
      adapterBinary: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/claude-acp/${REQUIRED_CLAUDE_ACP_VERSION}/node_modules/.bin/claude-agent-acp`,
    };
    const actualPaths = {
      configPath: runtime?.configPath,
      logDir: runtime?.logDir,
      signerPath: runtime?.signerPath,
      supervisorWorkingDirectory: runtime?.supervisorWorkingDirectory,
      adapterRoot: adapter?.root,
      adapterBinary: adapter?.binary,
    };
    for (const [label, expected] of Object.entries(expectedPaths)) {
      if (actualPaths[label] !== expected)
        errors.push(`Claude ${label} must use the launchd-safe Data-volume path`);
    }
    if (JSON.stringify(runtime?.node) !== JSON.stringify(REQUIRED_CLAUDE_NODE)) {
      errors.push("Claude Node runtime checkpoint drift");
    }
    if (claudeCode?.version !== REQUIRED_CLAUDE_CODE_VERSION) {
      errors.push(`Claude Code must be pinned to ${REQUIRED_CLAUDE_CODE_VERSION}`);
    }
    if (!isAbsoluteSafePath(claudeCode?.binary))
      errors.push("Claude Code binary must be an absolute safe path");
    if (!HEX_64.test(claudeCode?.binarySha256 ?? "")) {
      errors.push("Claude Code binary SHA-256 must be pinned");
    }
    if (claudeCode?.binarySha256 !== REQUIRED_CLAUDE_CODE_SHA256) {
      errors.push("Claude Code binary checkpoint drift");
    }
    if (claudeCode?.configDir !== undefined) {
      errors.push("Claude config directory override must be absent");
    }
    if (JSON.stringify(claudeCode?.auth) !== JSON.stringify(REQUIRED_CLAUDE_AUTH)) {
      errors.push("Claude auth must use the pinned Claude subscription login");
    }
  }
  if (selector === "cursor_cli") {
    const expectedPaths = {
      configPath: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/buzz/cursor-cli.toml`,
      logDir: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/logs`,
      signerPath: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/secrets/cursor-cli.sk`,
      supervisorWorkingDirectory: REQUIRED_CLAUDE_RUNTIME_ROOT,
      bootstrapPath: REQUIRED_CURSOR_BOOTSTRAP_PATH,
    };
    for (const [label, expected] of Object.entries(expectedPaths)) {
      if (runtime?.[label] !== expected)
        errors.push(`Cursor ${label} must use the launchd-safe Data-volume path`);
    }
    if (runtime?.bootstrapSha256 !== REQUIRED_CURSOR_BOOTSTRAP_SHA256) {
      errors.push("Cursor ACP bootstrap checkpoint drift");
    }
  }
  if (selector === "grok_cli") {
    const expectedPaths = {
      configPath: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/buzz/grok-cli.toml`,
      logDir: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/logs`,
      signerPath: `${REQUIRED_CLAUDE_RUNTIME_ROOT}/secrets/grok-cli.sk`,
      supervisorWorkingDirectory: REQUIRED_CLAUDE_RUNTIME_ROOT,
    };
    for (const [label, expected] of Object.entries(expectedPaths)) {
      if (runtime?.[label] !== expected) {
        errors.push(`Grok ${label} must use the launchd-safe Data-volume path`);
      }
    }
  }
  if (!(runtime?.path ?? []).every(isAbsoluteSafePath))
    errors.push("every PATH entry must be absolute and safe");
  if (!runtime?.path?.includes(path.dirname(adapter?.binary ?? ""))) {
    errors.push("PATH must include the pinned ACP adapter bin directory");
  }
  if (
    selector === "claude_cli" &&
    runtime?.path?.[0] !== path.dirname(runtime?.node?.binary ?? "")
  ) {
    errors.push("Claude PATH must resolve the pinned trusted Node runtime first");
  }

  const workspaces = manifest.workspaces;
  if (!workspaces?.allowed?.[workspaces?.default]) errors.push("default workspace must be allowed");
  for (const [name, workspacePath] of Object.entries(workspaces?.allowed ?? {})) {
    if (!/^[a-z0-9][a-z0-9-]*$/.test(name)) errors.push(`invalid workspace name: ${name}`);
    if (
      !isAbsoluteSafePath(workspacePath) ||
      !workspacePath.startsWith("/Volumes/AEON/Projects/")
    ) {
      errors.push(`${name}: workspace must be a bounded AEON project path`);
    }
  }

  const posture = manifest.posture;
  if (posture?.subscribe !== "config") errors.push("worker must use config subscriptions");
  if (posture?.respondTo !== "strict-allowlist") {
    errors.push("worker must use the strict inbound allowlist");
  }
  if (JSON.stringify(posture?.allowedRespondTo) !== JSON.stringify(["strict-allowlist"])) {
    errors.push("worker may only use strict-allowlist response mode");
  }
  if (posture?.dedup !== "queue" || posture?.multipleEventHandling !== "queue") {
    errors.push("queue semantics must remain enabled");
  }
  for (const field of ["presence", "typing", "relayObserver"]) {
    if (posture?.[field] !== true) errors.push(`${field} must remain enabled`);
  }
  const expectedMemory = selector !== "cursor_cli";
  if (posture?.memory !== expectedMemory) {
    errors.push(`memory must remain ${expectedMemory ? "enabled" : "disabled"}`);
  }
  if (posture?.basePrompt !== true && posture?.basePrompt !== false) {
    errors.push("basePrompt must be a boolean");
  }
  if (
    posture?.basePrompt === false &&
    !isAbsoluteSafePath(manifest.runtime?.systemPromptPath)
  ) {
    errors.push("a compact systemPromptPath is required when basePrompt is disabled");
  }
  if (
    manifest.runtime?.systemPromptPath !== undefined &&
    !HEX_64.test(manifest.runtime?.systemPromptSha256 ?? "")
  ) {
    errors.push("systemPromptSha256 must pin every configured system prompt");
  }
  const expectedPermissionMode = selector === "codex_cli" ? "default" : "bypass-permissions";
  if (posture?.permissionMode !== expectedPermissionMode) {
    errors.push(`${principal} Buzz permission mode must be ${expectedPermissionMode}`);
  }
  if (posture?.heartbeatIntervalSecs !== 0)
    errors.push("autonomous heartbeat prompts must remain off");
  if (manifest.supervisor?.runAtLoad !== false || manifest.supervisor?.keepAlive !== false) {
    errors.push("live activation must remain off");
  }

  return { ok: errors.length === 0, errors };
}

export function renderWorker(manifest, identityMap, workspaceName = manifest.workspaces.default) {
  const validation = validateManifest(manifest, identityMap);
  if (!validation.ok) throw new Error(validation.errors.join("\n"));

  const workspace = manifest.workspaces.allowed[workspaceName];
  if (!workspace) throw new Error(`workspace is not allowed: ${workspaceName}`);
  const selector = workerSelector(manifest);
  const principal = identityMap.members[manifest.worker.principal];
  const contract = WORKER_CONTRACTS[selector];
  const adapter = manifest.runtime[contract.adapterKey];
  const usesSafeSupervisor = true;
  const signerFile = manifest.runtime.signerPath;
  const allowlist = manifest.buzz.allowedInbound
    .filter((memberId) => memberId !== manifest.buzz.owner)
    .map((memberId) => memberPubkey(identityMap, memberId));
  const buzzArgs = [
    "--relay-url",
    manifest.buzz.relayUrl,
    "--private-key-file",
    signerFile,
    "--expected-public-key",
    principal.pubkey_hex,
    "--agent-owner",
    memberPubkey(identityMap, manifest.buzz.owner),
    "--agent-command",
    selector === "cursor_cli" ? `${adapter.root}/node` : adapter.binary,
    ...((adapter.args?.length ?? 0) > 0
      ? [
          `--agent-args=${
            selector === "cursor_cli"
              ? [
                  "--use-system-ca",
                  manifest.runtime.bootstrapPath,
                  workspace,
                  `${adapter.root}/index.js`,
                  ...adapter.args,
                ].join(",")
              : adapter.args.join(",")
          }`,
        ]
      : []),
    ...(usesSafeSupervisor ? ["--session-cwd", workspace] : []),
    ...(manifest.runtime.systemPromptPath
      ? ["--system-prompt-file", manifest.runtime.systemPromptPath]
      : []),
    ...(manifest.posture.basePrompt ? [] : ["--no-base-prompt"]),
    ...(manifest.posture.memory ? [] : ["--no-memory"]),
    ...(selector === "codex_cli"
      ? ["--model", adapter.model]
      : selector === "cursor_cli"
        ? ["--model", adapter.model.effective]
        : []),
    "--agent-publisher-credentials",
    "--agents",
    "1",
    "--subscribe",
    "config",
    "--config",
    manifest.runtime.configPath,
    "--respond-to",
    manifest.posture.respondTo,
    "--respond-to-allowlist",
    allowlist.join(","),
    "--allowed-respond-to",
    manifest.posture.allowedRespondTo.join(","),
    "--dedup",
    "queue",
    "--multiple-event-handling",
    "queue",
    "--relay-observer",
    "--permission-mode",
    manifest.posture.permissionMode,
    "--heartbeat-interval",
    String(manifest.posture.heartbeatIntervalSecs),
    "--turn-liveness-secs",
    String(manifest.posture.turnLivenessSecs),
    "--idle-timeout",
    String(manifest.posture.idleTimeoutSecs),
    "--max-turn-duration",
    String(manifest.posture.maxTurnDurationSecs),
    "--context-message-limit",
    String(manifest.posture.contextMessageLimit),
    "--max-turns-per-session",
    String(manifest.posture.maxTurnsPerSession),
  ];
  const claudeScrubPrefix = ANTHROPIC_CREDENTIAL_ENV.flatMap((name) => ["-u", name]);
  const cursorScrubPrefix = CURSOR_OVERRIDE_ENV.flatMap((name) => ["-u", name]);
  const grokScrubPrefix = GROK_OVERRIDE_ENV.flatMap((name) => ["-u", name]);
  const scrubPrefix =
    selector === "codex_cli"
      ? []
      : selector === "claude_cli"
      ? claudeScrubPrefix
      : selector === "cursor_cli"
        ? cursorScrubPrefix
        : grokScrubPrefix;
  return {
    enabled: false,
    label: manifest.worker.label,
    workspaceName,
    workingDirectory: manifest.runtime.supervisorWorkingDirectory,
    sessionCwd: workspace,
    subscriptionRoomIds: exactRoomIds(manifest, identityMap),
    command: usesSafeSupervisor ? ENV_BINARY : manifest.runtime.buzzAcpBinary,
    args: usesSafeSupervisor
      ? [...scrubPrefix, manifest.runtime.buzzAcpBinary, ...buzzArgs]
      : buzzArgs,
    environment:
      selector === "codex_cli"
        ? {
            PATH: manifest.runtime.path.join(":"),
            CODEX_HOME: manifest.runtime.codexHome,
            INITIAL_AGENT_MODE: manifest.runtime.initialAgentMode,
          }
        : selector === "claude_cli"
          ? {
              PATH: manifest.runtime.path.join(":"),
              CLAUDE_CODE_EXECUTABLE: manifest.runtime.claudeCode.binary,
            }
          : {
              ...(selector === "grok_cli" ? { HOME: "/Users/architect" } : {}),
              PATH: manifest.runtime.path.join(":"),
            },
    signerFile,
    expectedPublicKey: principal.pubkey_hex,
  };
}

function xml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

export function renderDisabledLaunchAgent(manifest, identityMap, workspaceName) {
  const worker = renderWorker(manifest, identityMap, workspaceName);
  const argvXml = [worker.command, ...worker.args]
    .map((value) => `    <string>${xml(value)}</string>`)
    .join("\n");
  const envXml = Object.entries(worker.environment)
    .map(([key, value]) => `    <key>${xml(key)}</key><string>${xml(value)}</string>`)
    .join("\n");
  const selector = workerSelector(manifest);
  const logRoot =
    manifest.runtime.logDir ?? `/Volumes/AEON/runtime/buzz/external-cli/${selector}/logs`;
  const logName = selector.replace("_", "-");

  return {
    ...worker,
    requiredDirectories: [
      ...new Set([
        path.dirname(manifest.runtime.configPath),
        ...(selector === "cursor_cli"
          ? [
              path.dirname(manifest.runtime.bootstrapPath),
              path.dirname(manifest.runtime.systemPromptPath),
            ]
          : []),
        logRoot,
        path.dirname(worker.signerFile),
        worker.workingDirectory,
        worker.sessionCwd,
      ]),
    ],
    runAtLoad: false,
    keepAlive: false,
    rollback: ["launchctl", "bootout", `gui/<uid>/${worker.label}`],
    plist: `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>${xml(worker.label)}</string>
  <key>ProgramArguments</key>
  <array>
${argvXml}
  </array>
  <key>WorkingDirectory</key><string>${xml(worker.workingDirectory)}</string>
  <key>EnvironmentVariables</key>
  <dict>
${envXml}
  </dict>
  <key>RunAtLoad</key><false/>
  <key>KeepAlive</key><false/>
  <key>ProcessType</key><string>Background</string>
  <key>StandardOutPath</key><string>${logRoot}/${logName}.log</string>
  <key>StandardErrorPath</key><string>${logRoot}/${logName}.err.log</string>
</dict>
</plist>
`,
  };
}

function exactTag(tags, expected) {
  return tags.filter(
    (tag) =>
      tag.length === expected.length && tag.every((value, index) => value === expected[index]),
  );
}

export function correlateVerifiedReceipt({
  requestEventId,
  channelId,
  replyEvent,
  observerRun,
  expectedPubkey,
}) {
  if (!HEX_64.test(requestEventId) || !HEX_64.test(expectedPubkey)) {
    throw new Error("request and signer ids must be 64 lowercase hex");
  }
  if (replyEvent?.verified !== true) throw new Error("reply signature must be verified");
  if (
    replyEvent?.kind !== 9 ||
    replyEvent?.pubkey !== expectedPubkey ||
    !HEX_64.test(replyEvent?.id ?? "")
  ) {
    throw new Error("reply identity mismatch");
  }
  if (exactTag(replyEvent.tags ?? [], ["h", channelId]).length !== 1) {
    throw new Error("reply requires one exact channel tag");
  }
  if (exactTag(replyEvent.tags ?? [], ["e", requestEventId, "", "reply"]).length !== 1) {
    throw new Error("reply requires one exact request anchor");
  }
  if (
    observerRun?.requestEventId !== requestEventId ||
    observerRun?.replyEventId !== replyEvent.id ||
    observerRun?.channelId !== channelId ||
    !observerRun?.sessionId ||
    !observerRun?.runId
  ) {
    throw new Error("observer run correlation mismatch");
  }
  return {
    requestEventId,
    replyEventId: replyEvent.id,
    sessionId: observerRun.sessionId,
    runId: observerRun.runId,
    channelId,
  };
}
