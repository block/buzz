import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import test from "node:test";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  REQUIRED_ROOM_NAMES,
  correlateVerifiedReceipt,
  hashCursorClosure,
  hashPackageClosure,
  loadJson,
  renderDisabledLaunchAgent,
  renderWorker,
  validateAmbientAnthropicCredentials,
  validateAmbientCursorOverrides,
  validateAmbientGrokOverrides,
  validateClaudeSubscriptionAuth,
  validateCursorSubscriptionAuth,
  validateManifest,
  validatePinnedNodeRuntime,
  validateSubscriptionProjection,
} from "./worker.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const manifest = loadJson(join(here, "manifest.json"));
const claudeManifest = loadJson(join(here, "manifest.claude_cli.json"));
const cursorManifest = loadJson(join(here, "manifest.cursor_cli.json"));
const grokManifest = loadJson(join(here, "manifest.grok_cli.json"));
const identityMap = loadJson(join(here, "fixtures", "identity-map.json"));
const codexConfig = readFileSync(join(here, "config", "codex_cli.toml"), "utf8");

test("manifest binds external codex_cli identity without changing Aspect semantics", () => {
  const result = validateManifest(manifest, identityMap);
  assert.deepEqual(result.errors, []);
  assert.equal(result.ok, true);
  assert.equal(identityMap.members.codex_cli.gateway_agent_id, null);
  assert.equal(identityMap.members.codex_cli.aspect_slug, null);
});

test("all external workers pin the same shared Data-volume buzz-acp release", () => {
  const binary = "/Users/architect/Library/Application Support/AEON/aeon-v6/bin/buzz-acp";
  const sha256 = "1d260060a0b790645a0455d23c7a82ac7836193108673a76f44423c5d81be9be";
  assert.equal(manifest.runtime.buzzAcpBinary, binary);
  assert.equal(claudeManifest.runtime.buzzAcpBinary, binary);
  assert.equal(cursorManifest.runtime.buzzAcpBinary, binary);
  assert.equal(grokManifest.runtime.buzzAcpBinary, binary);
  assert.equal(manifest.runtime.buzzAcpSha256, sha256);
  assert.equal(claudeManifest.runtime.buzzAcpSha256, sha256);
  assert.equal(cursorManifest.runtime.buzzAcpSha256, sha256);
  assert.equal(grokManifest.runtime.buzzAcpSha256, sha256);
});

test("claude_cli selector binds the established external claude_code identity", () => {
  const result = validateManifest(claudeManifest, identityMap);
  assert.deepEqual(result.errors, []);
  assert.equal(result.ok, true);
  assert.equal(claudeManifest.worker.selector, "claude_cli");
  assert.equal(claudeManifest.worker.principal, "claude_code");
  assert.equal(identityMap.members.claude_code.gateway_agent_id, null);
  assert.equal(identityMap.members.claude_code.aspect_slug, null);
  assert.notEqual(
    identityMap.members.claude_code.pubkey_hex,
    identityMap.members.codex_cli.pubkey_hex,
  );
});

test("cursor_cli selector binds the established external Cursor identity", () => {
  const result = validateManifest(cursorManifest, identityMap);
  assert.deepEqual(result.errors, []);
  assert.equal(result.ok, true);
  assert.equal(cursorManifest.worker.selector, "cursor_cli");
  assert.equal(cursorManifest.worker.principal, "cursor_cli");
  assert.equal(identityMap.members.cursor_cli.gateway_agent_id, null);
  assert.equal(identityMap.members.cursor_cli.aspect_slug, null);
  assert.notEqual(
    identityMap.members.cursor_cli.pubkey_hex,
    identityMap.members.codex_cli.pubkey_hex,
  );
});

test("grok_cli selector binds a distinct external Grok identity", () => {
  const result = validateManifest(grokManifest, identityMap);
  assert.deepEqual(result.errors, []);
  assert.equal(result.ok, true);
  assert.equal(grokManifest.worker.selector, "grok_cli");
  assert.equal(grokManifest.worker.principal, "grok_cli");
  assert.equal(identityMap.members.grok_cli.gateway_agent_id, null);
  assert.equal(identityMap.members.grok_cli.aspect_slug, null);
  assert.notEqual(
    identityMap.members.grok_cli.pubkey_hex,
    identityMap.members.codex_cli.pubkey_hex,
  );
  assert.notEqual(
    identityMap.members.grok_cli.pubkey_hex,
    identityMap.members.cursor_cli.pubkey_hex,
  );
});

test("Codex launch argv binds publisher credentials, identity, and all eight rooms", () => {
  const worker = renderWorker(manifest, identityMap);
  assert.equal(
    worker.environment.PATH.split(":")[0],
    "/Users/architect/.nvm/versions/node/v24.1.0/bin",
  );
  assert.equal(worker.args.includes("--no-agent-publisher-credentials"), false);
  assert.equal(worker.args.filter((arg) => arg === "--agent-publisher-credentials").length, 1);
  assert.equal(worker.args.includes("--private-key"), false);
  assert.equal(worker.args.includes("--private-key-file"), true);
  assert.equal(worker.args.includes("--relay-observer"), true);
  assert.equal(
    worker.args[worker.args.indexOf("--private-key-file") + 1],
    manifest.runtime.signerPath,
  );
  assert.equal(
    worker.args[worker.args.indexOf("--expected-public-key") + 1],
    identityMap.members.codex_cli.pubkey_hex,
  );
  assert.equal(worker.args[worker.args.indexOf("--subscribe") + 1], manifest.posture.subscribe);
  assert.equal(worker.args[worker.args.indexOf("--config") + 1], manifest.runtime.configPath);
  assert.deepEqual(
    worker.subscriptionRoomIds,
    [...manifest.buzz.sharedRooms, ...manifest.buzz.officeRooms].map(
      (roomName) => identityMap.channels[roomName].channel_id,
    ),
  );
  assert.equal(worker.subscriptionRoomIds.length, 8);
  assert.equal(new Set(worker.subscriptionRoomIds).size, 8);
  assert.equal(worker.environment.BUZZ_PRIVATE_KEY, undefined);
  assert.equal(worker.environment.BUZZ_RELAY_URL, undefined);
});

test("subscription validator requires the exact source-projected room set", () => {
  assert.deepEqual(
    [...manifest.buzz.sharedRooms, ...manifest.buzz.officeRooms],
    REQUIRED_ROOM_NAMES,
  );
  const result = validateSubscriptionProjection(codexConfig, manifest, identityMap);
  assert.equal(result.ok, true);
  assert.deepEqual(result.errors, []);
  assert.deepEqual(result.roomIds, renderWorker(manifest, identityMap).subscriptionRoomIds);

  const duplicateRoom = codexConfig.replace(result.roomIds.at(-1), result.roomIds[0]);
  assert.match(
    validateSubscriptionProjection(duplicateRoom, manifest, identityMap).errors.join("\n"),
    /exactly the eight canonical rooms/,
  );

  const extraRoom = codexConfig.replace(
    result.roomIds.at(-1),
    "ffffffff-ffff-4fff-8fff-ffffffffffff",
  );
  assert.match(
    validateSubscriptionProjection(extraRoom, manifest, identityMap).errors.join("\n"),
    /exactly the eight canonical rooms/,
  );

  const channelBlocks = [...codexConfig.matchAll(/^\s*channels\s*=\s*(\[[\s\S]*?^\s*\])/gm)].map(
    (match) => match[0],
  );
  const swappedArrays = codexConfig
    .replace(channelBlocks[0], "__FIRST_CHANNEL_ARRAY__")
    .replace(channelBlocks[1], channelBlocks[0])
    .replace("__FIRST_CHANNEL_ARRAY__", channelBlocks[1]);
  assert.match(
    validateSubscriptionProjection(swappedArrays, manifest, identityMap).errors.join("\n"),
    /exactly the eight canonical rooms/,
  );

  const officeDrift = structuredClone(manifest);
  officeDrift.buzz.officeRooms[5] = "ops";
  assert.match(
    validateManifest(officeDrift, identityMap).errors.join("\n"),
    /six canonical Aspect offices/,
  );

  const extraRule = `${codexConfig}
[[rules]]
name = "unrestricted"
kinds = [9]
require_mention = false
`;
  assert.match(
    validateSubscriptionProjection(extraRule, manifest, identityMap).errors.join("\n"),
    /exactly two rules/,
  );

  const misplacedMention = `require_mention = true
${codexConfig.replace("require_mention = true", "")}`;
  assert.match(
    validateSubscriptionProjection(misplacedMention, manifest, identityMap).errors.join("\n"),
    /each subscription rule must require a mention/,
  );

  const firstChannels = codexConfig.match(/^\s*channels\s*=\s*(\[[\s\S]*?^\s*\])/m)[0];
  const misplacedTableFields = codexConfig
    .replace(firstChannels, "")
    .replace("require_mention = true", `[other]\n${firstChannels}\nrequire_mention = true`);
  assert.match(
    validateSubscriptionProjection(misplacedTableFields, manifest, identityMap).errors.join("\n"),
    /each subscription rule must contain exactly one channel array/,
  );

  const commentedTableBoundary = codexConfig.replace(
    "require_mention = true",
    "[other] # valid trailing comment\nrequire_mention = true",
  );
  assert.match(
    validateSubscriptionProjection(commentedTableBoundary, manifest, identityMap).errors.join("\n"),
    /each subscription rule must require a mention/,
  );

  const multilineBypass = codexConfig
    .replace(firstChannels, `channels = "all"\nignored = """\n${firstChannels}`)
    .replace("require_mention = true", 'require_mention = true\n"""');
  assert.match(
    validateSubscriptionProjection(multilineBypass, manifest, identityMap).errors.join("\n"),
    /must not contain multiline strings/,
  );

  const duplicateName = codexConfig.replace(
    'name = "aeon-aspect-offices"',
    'name = "aeon-shared-control"',
  );
  assert.match(
    validateSubscriptionProjection(duplicateName, manifest, identityMap).errors.join("\n"),
    /canonical shared-control and Aspect-office rules/,
  );

  const malformedHeader = codexConfig.replace("[[rules]]", "[[rules]]garbage");
  assert.match(
    validateSubscriptionProjection(malformedHeader, manifest, identityMap).errors.join("\n"),
    /exactly two rules/,
  );

  const unknownPreamble = `unexpected = true\n${codexConfig}`;
  assert.match(
    validateSubscriptionProjection(unknownPreamble, manifest, identityMap).errors.join("\n"),
    /must not contain content outside the two canonical rules/,
  );

  const unknownTable = `${codexConfig}\n[unexpected]\nvalue = true\n`;
  assert.match(
    validateSubscriptionProjection(unknownTable, manifest, identityMap).errors.join("\n"),
    /must not contain content outside the two canonical rules/,
  );
});

test("renderer pins one full-access codex-acp subprocess", () => {
  const worker = renderWorker(manifest, identityMap);
  assert.equal(worker.command, "/usr/bin/env");
  assert.equal(worker.environment.INITIAL_AGENT_MODE, "agent-full-access");
  assert.equal(
    worker.environment.CODEX_HOME,
    "/Users/architect/Library/Application Support/AEON/aeon-v6/codex-home",
  );
  assert.equal(worker.args[worker.args.indexOf("--agents") + 1], "1");
  assert.equal(worker.args[worker.args.indexOf("--permission-mode") + 1], "default");
  assert.equal(
    worker.args[worker.args.indexOf("--agent-command") + 1],
    manifest.runtime.codexAcp.binary,
  );
  assert.equal(
    worker.args[worker.args.indexOf("--system-prompt-file") + 1],
    manifest.runtime.systemPromptPath,
  );
  assert.match(manifest.runtime.systemPromptSha256, /^[0-9a-f]{64}$/);
  assert.equal(worker.args[worker.args.indexOf("--model") + 1], manifest.runtime.codexAcp.model);
});

test("renderer pins one Claude ACP subprocess and installed Claude Code", () => {
  const worker = renderWorker(claudeManifest, identityMap);
  assert.equal(worker.command, "/usr/bin/env");
  assert.deepEqual(worker.args.slice(0, 5), [
    "-u",
    "ANTHROPIC_API_KEY",
    "-u",
    "ANTHROPIC_AUTH_TOKEN",
    claudeManifest.runtime.buzzAcpBinary,
  ]);
  assert.equal(worker.args.includes("--agent-publisher-credentials"), true);
  assert.equal(worker.args.includes("--no-agent-publisher-credentials"), false);
  assert.equal(
    worker.args[worker.args.indexOf("--agent-command") + 1],
    "/Users/architect/Library/Application Support/AEON/aeon-v6/claude-acp/0.62.0/node_modules/.bin/claude-agent-acp",
  );
  assert.equal(worker.args[worker.args.indexOf("--agents") + 1], "1");
  assert.equal(worker.args[worker.args.indexOf("--permission-mode") + 1], "bypass-permissions");
  assert.equal(
    worker.args[worker.args.indexOf("--agent-command") + 1],
    claudeManifest.runtime.claudeAcp.binary,
  );
  assert.equal(
    worker.signerFile,
    "/Users/architect/Library/Application Support/AEON/aeon-v6/secrets/claude-code.sk",
  );
  assert.equal(worker.expectedPublicKey, identityMap.members.claude_code.pubkey_hex);
  assert.equal(
    worker.environment.CLAUDE_CODE_EXECUTABLE,
    "/Users/architect/.local/share/claude/versions/2.1.220",
  );
  assert.equal(worker.environment.CLAUDE_CONFIG_DIR, undefined);
  assert.equal(worker.environment.ANTHROPIC_API_KEY, undefined);
  assert.equal(worker.environment.ANTHROPIC_AUTH_TOKEN, undefined);
  assert.equal(worker.environment.RUST_LOG, undefined);
  assert.equal(
    worker.environment.PATH.split(":")[0],
    "/Users/architect/.nvm/versions/node/v24.1.0/bin",
  );
  assert.equal(
    worker.environment.PATH.includes(
      "/Volumes/AEON/runtime/aeon-v6-state/service-runtime/current/bin",
    ),
    false,
  );
});

test("renderer pins one native Cursor ACP subprocess with a proven operational model", () => {
  const worker = renderWorker(cursorManifest, identityMap);
  const adapter = cursorManifest.runtime.cursorAcp;
  assert.equal(worker.command, "/usr/bin/env");
  assert.deepEqual(worker.args.slice(0, 5), [
    "-u",
    "CURSOR_API_KEY",
    "-u",
    "CURSOR_API_ENDPOINT",
    cursorManifest.runtime.buzzAcpBinary,
  ]);
  assert.equal(
    worker.args[worker.args.indexOf("--agent-command") + 1],
    `${adapter.root}/node`,
  );
  assert.deepEqual(
    worker.args.filter((value) => value.startsWith("--agent-args=")),
    [
      `--agent-args=--use-system-ca,${cursorManifest.runtime.bootstrapPath},/Volumes/AEON/Projects/aeon-v6,${adapter.root}/index.js,--trust,acp`,
    ],
  );
  assert.equal(worker.args.includes("--agent-args"), false);
  assert.equal(
    worker.args[worker.args.lastIndexOf("--model") + 1],
    "grok-4.5[effort=high,fast=true]",
  );
  assert.equal(
    worker.args[worker.args.indexOf("--system-prompt-file") + 1],
    cursorManifest.runtime.systemPromptPath,
  );
  assert.match(cursorManifest.runtime.systemPromptSha256, /^[0-9a-f]{64}$/);
  assert.equal(worker.args.includes("--no-base-prompt"), true);
  assert.equal(worker.args.includes("--no-memory"), true);
  assert.equal(
    worker.args[worker.args.indexOf("--context-message-limit") + 1],
    "0",
  );
  assert.doesNotMatch(
    worker.args.find((value) => value.startsWith("--agent-args=")),
    /--model/,
  );
  assert.equal(adapter.model.requested, "cursor-grok-4.5-high");
  assert.equal(adapter.model.effective, "grok-4.5[effort=high,fast=true]");
  assert.equal(adapter.model.selectionStatus, "upstream_limited_to_fast_wire_variant");
  assert.equal(
    worker.args[worker.args.indexOf("--session-cwd") + 1],
    "/Volumes/AEON/Projects/aeon-v6",
  );
  assert.equal(worker.args[worker.args.indexOf("--permission-mode") + 1], "bypass-permissions");
  assert.equal(worker.environment.CURSOR_API_KEY, undefined);
  assert.equal(worker.environment.CURSOR_API_ENDPOINT, undefined);
  assert.equal(worker.signerFile, cursorManifest.runtime.signerPath);
  assert.equal(worker.expectedPublicKey, identityMap.members.cursor_cli.pubkey_hex);
});

test("Codex and Claude omit --agent-args when adapters have no child args", () => {
  for (const worker of [
    renderWorker(manifest, identityMap),
    renderWorker(claudeManifest, identityMap),
  ]) {
    assert.equal(
      worker.args.some(
        (value) => value === "--agent-args" || value.startsWith("--agent-args="),
      ),
      false,
    );
  }
});

test("renderer pins one native Grok ACP subprocess with full coding authority", () => {
  const worker = renderWorker(grokManifest, identityMap);
  assert.equal(worker.command, "/usr/bin/env");
  assert.equal(worker.sessionCwd, "/Volumes/AEON/Projects/aeon-v6");
  assert.equal(worker.environment.HOME, "/Users/architect");
  assert.equal(worker.environment.PATH.startsWith("/Users/architect/.grok/bin:"), true);
  assert.equal(worker.environment.PATH.includes("/Users/architect/.local/bin"), true);
  const buzzPublisher = worker.environment.PATH.split(":")
    .map((directory) => join(directory, "buzz"))
    .find((candidate) => existsSync(candidate));
  assert.equal(buzzPublisher, "/Users/architect/.local/bin/buzz");
  assert.equal(
    worker.args.find((value) => value.startsWith("--agent-args=")),
    `--agent-args=${grokManifest.runtime.grokAcp.args.join(",")}`,
  );
  assert.equal(worker.args.includes("--agent-args"), false);
  // Regression: Grok model/reasoning stay inside the single packed --agent-args=
  // token; buzz-acp must not see them as top-level flags (Cursor alone uses --model).
  assert.equal(
    worker.args.includes("--model") || worker.args.includes("--reasoning-effort"),
    false,
  );
  assert.equal(worker.args[worker.args.indexOf("--permission-mode") + 1], "bypass-permissions");
  assert.equal(worker.args.filter((arg) => arg === "--agent-publisher-credentials").length, 1);
  assert.equal(worker.expectedPublicKey, identityMap.members.grok_cli.pubkey_hex);
  assert.deepEqual(
    worker.subscriptionRoomIds,
    renderWorker(manifest, identityMap).subscriptionRoomIds,
  );
});

test("Claude rendered command scrubs ambient API credentials from its child", () => {
  const worker = renderWorker(claudeManifest, identityMap);
  const buzzBinaryIndex = worker.args.indexOf(claudeManifest.runtime.buzzAcpBinary);
  const probe = spawnSync(
    worker.command,
    [
      ...worker.args.slice(0, buzzBinaryIndex),
      process.execPath,
      "-e",
      "process.stdout.write(JSON.stringify({key:process.env.ANTHROPIC_API_KEY,token:process.env.ANTHROPIC_AUTH_TOKEN}))",
    ],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        ANTHROPIC_API_KEY: "ambient-key",
        ANTHROPIC_AUTH_TOKEN: "ambient-token",
      },
    },
  );
  assert.equal(probe.status, 0, probe.stderr);
  assert.deepEqual(JSON.parse(probe.stdout), {});
});

test("renderer pins Architect and all six Aspects as inbound authority", () => {
  const expectedAllowlist = [
    identityMap.members.nexus.pubkey_hex,
    identityMap.members.mechanon.pubkey_hex,
    identityMap.members.fontis.pubkey_hex,
    identityMap.members.sapientis.pubkey_hex,
    identityMap.members.viatica.pubkey_hex,
    identityMap.members.voxis.pubkey_hex,
  ];
  for (const workerManifest of [manifest, claudeManifest, cursorManifest, grokManifest]) {
    const worker = renderWorker(workerManifest, identityMap);
    const allowlist = worker.args[worker.args.indexOf("--respond-to-allowlist") + 1].split(",");
    assert.deepEqual(allowlist, expectedAllowlist);
    assert.equal(
      worker.args[worker.args.indexOf("--agent-owner") + 1],
      identityMap.members.architect.pubkey_hex,
    );
    assert.equal(worker.args[worker.args.indexOf("--respond-to") + 1], "strict-allowlist");
    assert.equal(
      worker.args[worker.args.indexOf("--allowed-respond-to") + 1],
      "strict-allowlist",
    );
    for (const externalSeat of ["codex_cli", "claude_code", "cursor_cli", "grok_cli"]) {
      assert.equal(allowlist.includes(identityMap.members[externalSeat].pubkey_hex), false);
    }
  }
});

test("renderer rejects missing or external-seat inbound authority", () => {
  const missingAspect = structuredClone(manifest);
  missingAspect.buzz.allowedInbound = missingAspect.buzz.allowedInbound.filter(
    (memberId) => memberId !== "voxis",
  );
  assert.throws(
    () => renderWorker(missingAspect, identityMap),
    /inbound allowlist must be exactly Architect and the six canonical Aspects/,
  );

  const externalSeat = structuredClone(manifest);
  externalSeat.buzz.allowedInbound = [...externalSeat.buzz.allowedInbound, "cursor_cli"];
  assert.throws(
    () => renderWorker(externalSeat, identityMap),
    /inbound allowlist must be exactly Architect and the six canonical Aspects/,
  );
});

test("renderer rejects colliding authorities and Aspects absent from their offices", () => {
  const collidingIdentityMap = structuredClone(identityMap);
  collidingIdentityMap.members.fontis.pubkey_hex =
    collidingIdentityMap.members.cursor_cli.pubkey_hex;
  assert.throws(
    () => renderWorker(manifest, collidingIdentityMap),
    /Architect, Aspect, and external CLI pubkeys must be unique/,
  );

  const missingOfficeMember = structuredClone(identityMap);
  missingOfficeMember.channels.aspect_fontis.members =
    missingOfficeMember.channels.aspect_fontis.members.filter(
      (memberId) => memberId !== "fontis",
    );
  assert.throws(
    () => renderWorker(manifest, missingOfficeMember),
    /aspect_fontis: fontis is not a member/,
  );

  const missingSharedRoomMember = structuredClone(identityMap);
  missingSharedRoomMember.channels.concilium.members =
    missingSharedRoomMember.channels.concilium.members.filter(
      (memberId) => memberId !== "fontis",
    );
  assert.throws(
    () => renderWorker(manifest, missingSharedRoomMember),
    /concilium: fontis is not a member/,
  );
});

test("workspace selection is bounded to the manifest allowlist", () => {
  const codexWorker = renderWorker(manifest, identityMap, "buzz");
  assert.equal(
    codexWorker.workingDirectory,
    "/Users/architect/Library/Application Support/AEON/aeon-v6",
  );
  assert.equal(codexWorker.sessionCwd, "/Volumes/AEON/Projects/buzz");
  assert.equal(
    codexWorker.args[codexWorker.args.indexOf("--session-cwd") + 1],
    "/Volumes/AEON/Projects/buzz",
  );
  assert.throws(() => renderWorker(manifest, identityMap, "/tmp/escape"), /not allowed/);
  const claudeWorker = renderWorker(claudeManifest, identityMap, "codex");
  assert.equal(
    claudeWorker.workingDirectory,
    "/Users/architect/Library/Application Support/AEON/aeon-v6",
  );
  assert.equal(claudeWorker.sessionCwd, "/Volumes/AEON/Projects/codex");
  assert.equal(
    claudeWorker.args[claudeWorker.args.indexOf("--session-cwd") + 1],
    "/Volumes/AEON/Projects/codex",
  );
  const cursorWorker = renderWorker(cursorManifest, identityMap, "buzz");
  assert.equal(
    cursorWorker.workingDirectory,
    "/Users/architect/Library/Application Support/AEON/aeon-v6",
  );
  assert.equal(cursorWorker.sessionCwd, "/Volumes/AEON/Projects/buzz");
  assert.equal(
    cursorWorker.args.find((value) => value.startsWith("--agent-args=")),
    `--agent-args=--use-system-ca,${cursorManifest.runtime.bootstrapPath},/Volumes/AEON/Projects/buzz,${cursorManifest.runtime.cursorAcp.root}/index.js,--trust,acp`,
  );
  assert.equal(
    cursorWorker.args[cursorWorker.args.indexOf("--session-cwd") + 1],
    "/Volumes/AEON/Projects/buzz",
  );
  const grokWorker = renderWorker(grokManifest, identityMap, "aeon-v6");
  assert.equal(
    grokWorker.workingDirectory,
    "/Users/architect/Library/Application Support/AEON/aeon-v6",
  );
  assert.equal(
    grokWorker.args.find((value) => value.startsWith("--agent-args=")),
    `--agent-args=${grokManifest.runtime.grokAcp.args.join(",")}`,
  );
});

test("launchd artifact remains inert and secret-free", () => {
  const artifact = renderDisabledLaunchAgent(manifest, identityMap);
  assert.equal(artifact.runAtLoad, false);
  assert.equal(artifact.keepAlive, false);
  assert.match(artifact.plist, /<key>RunAtLoad<\/key><false\/>/);
  assert.match(artifact.plist, /<key>KeepAlive<\/key><false\/>/);
  assert.match(artifact.plist, /INITIAL_AGENT_MODE<\/key><string>agent-full-access/);
  assert.doesNotMatch(artifact.plist, /BUZZ_PRIVATE_KEY|nsec1/);
  assert.deepEqual(artifact.requiredDirectories, [
    "/Users/architect/Library/Application Support/AEON/aeon-v6/buzz",
    "/Users/architect/Library/Application Support/AEON/aeon-v6/logs",
    "/Users/architect/Library/Application Support/AEON/aeon-v6/secrets",
    "/Users/architect/Library/Application Support/AEON/aeon-v6",
    "/Volumes/AEON/Projects/aeon-v6",
  ]);
});

test("Claude launchd artifact is separate, inert, and secret-free", () => {
  const artifact = renderDisabledLaunchAgent(claudeManifest, identityMap);
  assert.equal(artifact.label, "org.aeon.buzz-acp.claude-cli");
  assert.equal(artifact.runAtLoad, false);
  assert.equal(artifact.keepAlive, false);
  assert.match(artifact.plist, /CLAUDE_CODE_EXECUTABLE/);
  assert.match(artifact.plist, /<string>-u<\/string>\s+<string>ANTHROPIC_API_KEY<\/string>/);
  assert.match(artifact.plist, /<string>-u<\/string>\s+<string>ANTHROPIC_AUTH_TOKEN<\/string>/);
  assert.doesNotMatch(
    artifact.plist,
    /<key>ANTHROPIC_API_KEY|<key>ANTHROPIC_AUTH_TOKEN|CLAUDE_CONFIG_DIR|nsec1|sk-ant-/,
  );
  assert.deepEqual(artifact.requiredDirectories, [
    "/Users/architect/Library/Application Support/AEON/aeon-v6/buzz",
    "/Users/architect/Library/Application Support/AEON/aeon-v6/logs",
    "/Users/architect/Library/Application Support/AEON/aeon-v6/secrets",
    "/Users/architect/Library/Application Support/AEON/aeon-v6",
    "/Volumes/AEON/Projects/aeon-v6",
  ]);
  assert.match(
    artifact.plist,
    /\/Users\/architect\/Library\/Application Support\/AEON\/aeon-v6\/bin\/buzz-acp/,
  );
  assert.doesNotMatch(artifact.plist, /buzz-acp-claude-cli/);
  assert.doesNotMatch(artifact.plist, /\/Volumes\/AEON\/runtime\/buzz\/external-cli\/claude_cli/);
  assert.match(
    artifact.plist,
    /\/Users\/architect\/Library\/Application Support\/AEON\/aeon-v6\/secrets\/claude-code\.sk/,
  );
  assert.match(
    artifact.plist,
    /<key>WorkingDirectory<\/key><string>\/Users\/architect\/Library\/Application Support\/AEON\/aeon-v6<\/string>/,
  );
  assert.match(artifact.plist, /\/Users\/architect\/\.nvm\/versions\/node\/v24\.1\.0\/bin/);
  assert.match(
    artifact.plist,
    /<string>--session-cwd<\/string>\s+<string>\/Volumes\/AEON\/Projects\/aeon-v6<\/string>/,
  );
  assert.doesNotMatch(artifact.plist, /RUST_LOG/);
  assert.doesNotMatch(
    artifact.plist,
    /\/Volumes\/AEON\/Projects\/buzz-data\/keys\/claude_code\.sk/,
  );
});

test("Cursor launchd artifact is separate, inert, and secret-free", () => {
  const artifact = renderDisabledLaunchAgent(cursorManifest, identityMap);
  assert.equal(artifact.label, "org.aeon.buzz-acp.cursor-cli");
  assert.equal(artifact.runAtLoad, false);
  assert.equal(artifact.keepAlive, false);
  assert.match(artifact.plist, /<string>-u<\/string>\s+<string>CURSOR_API_KEY<\/string>/);
  assert.match(artifact.plist, /<string>-u<\/string>\s+<string>CURSOR_API_ENDPOINT<\/string>/);
  assert.doesNotMatch(artifact.plist, /<key>CURSOR_API_KEY|<key>CURSOR_API_ENDPOINT|nsec1/);
  assert.match(artifact.plist, /grok-4\.5\[effort=high,fast=true\]/);
  assert.match(artifact.plist, /cursor-cli-system\.md/);
  assert.match(artifact.plist, /<string>--no-base-prompt<\/string>/);
  assert.match(artifact.plist, /<string>--session-cwd<\/string>/);
  assert.match(
    artifact.plist,
    /\/Users\/architect\/\.local\/share\/cursor-agent\/versions\/2026\.07\.23-e383d2b\/node/,
  );
  assert.match(artifact.plist, /cursor-acp-bootstrap\.cjs/);
  assert.match(
    artifact.plist,
    /--agent-args=--use-system-ca,\/Users\/architect\/Library\/Application Support\/AEON\/aeon-v6\/buzz\/cursor-acp-bootstrap\.cjs,\/Volumes\/AEON\/Projects\/aeon-v6,\/Users\/architect\/\.local\/share\/cursor-agent\/versions\/2026\.07\.23-e383d2b\/index\.js,--trust,acp/,
  );
  assert.match(
    artifact.plist,
    /<key>WorkingDirectory<\/key><string>\/Users\/architect\/Library\/Application Support\/AEON\/aeon-v6<\/string>/,
  );
  assert.deepEqual(artifact.requiredDirectories, [
    "/Users/architect/Library/Application Support/AEON/aeon-v6/buzz",
    "/Users/architect/Library/Application Support/AEON/aeon-v6/logs",
    "/Users/architect/Library/Application Support/AEON/aeon-v6/secrets",
    "/Users/architect/Library/Application Support/AEON/aeon-v6",
    "/Volumes/AEON/Projects/aeon-v6",
  ]);
});

test("Cursor ACP bootstrap is isolated from the other CLI seats", () => {
  for (const workerManifest of [manifest, claudeManifest, grokManifest]) {
    const worker = renderWorker(workerManifest, identityMap);
    assert.equal(worker.command, "/usr/bin/env");
    assert.equal(worker.args.includes(cursorManifest.runtime.bootstrapPath), false);
  }
});

test("Grok launchd artifact is separate, inert, and scrubs auth overrides", () => {
  const artifact = renderDisabledLaunchAgent(grokManifest, identityMap);
  assert.equal(artifact.label, "org.aeon.buzz-acp.grok-cli");
  assert.equal(artifact.runAtLoad, false);
  assert.equal(artifact.keepAlive, false);
  for (const name of ["XAI_API_KEY", "GROK_AUTH", "GROK_HOME", "XAI_API_BASE_URL"]) {
    assert.match(artifact.plist, new RegExp(`<string>-u<\\/string>\\s+<string>${name}<\\/string>`));
  }
  assert.doesNotMatch(artifact.plist, /<key>XAI_API_KEY|<key>GROK_AUTH|<key>GROK_HOME|nsec1/);
  assert.match(
    artifact.plist,
    /<string>--agent-args=agent,--model,grok-4\.5,--reasoning-effort,high,--always-approve,stdio<\/string>/,
  );
});

test("Claude and Cursor/Grok launch projections retain their current behavior", () => {
  const claude = renderWorker(claudeManifest, identityMap);
  assert.deepEqual(
    {
      command: claude.command,
      principal: claude.expectedPublicKey,
      permissionMode: claude.args[claude.args.indexOf("--permission-mode") + 1],
      publisherFlagCount: claude.args.filter((arg) => arg === "--agent-publisher-credentials")
        .length,
      rooms: claude.subscriptionRoomIds,
    },
    {
      command: "/usr/bin/env",
      principal: identityMap.members.claude_code.pubkey_hex,
      permissionMode: "bypass-permissions",
      publisherFlagCount: 1,
      rooms: renderWorker(manifest, identityMap).subscriptionRoomIds,
    },
  );

  const cursor = renderWorker(cursorManifest, identityMap);
  assert.deepEqual(
    {
      command: cursor.command,
      principal: cursor.expectedPublicKey,
      model: cursorManifest.runtime.cursorAcp.model.effective,
      permissionMode: cursor.args[cursor.args.indexOf("--permission-mode") + 1],
      publisherFlagCount: cursor.args.filter((arg) => arg === "--agent-publisher-credentials")
        .length,
      rooms: cursor.subscriptionRoomIds,
    },
    {
      command: "/usr/bin/env",
      principal: identityMap.members.cursor_cli.pubkey_hex,
      model: "grok-4.5[effort=high,fast=true]",
      permissionMode: "bypass-permissions",
      publisherFlagCount: 1,
      rooms: renderWorker(manifest, identityMap).subscriptionRoomIds,
    },
  );
});

test("Codex plist, validator summary, and validator log expose no secrets", () => {
  const sentinelSecret = "nsec1-validator-secret-sentinel";
  const artifact = renderDisabledLaunchAgent(manifest, identityMap);
  const validation = spawnSync(
    process.execPath,
    [join(here, "validate.mjs"), "--worker", "codex_cli"],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        BUZZ_PRIVATE_KEY: sentinelSecret,
      },
    },
  );
  assert.equal(validation.status, 0, validation.stderr);
  const summary = JSON.parse(validation.stdout);
  assert.equal(summary.principal, "codex_cli");
  assert.equal(summary.agentMode, "agent-full-access");
  assert.equal(summary.roomCount, 8);
  assert.equal(summary.publisherCredentials, "managed");

  for (const output of [artifact.plist, validation.stdout, validation.stderr]) {
    assert.doesNotMatch(output, /BUZZ_PRIVATE_KEY|nsec1/);
    assert.equal(output.includes(sentinelSecret), false);
  }
});

test("Claude authority contract rejects missing identity and mode drift", () => {
  const missingIdentity = structuredClone(identityMap);
  delete missingIdentity.members.claude_code;
  assert.match(
    validateManifest(claudeManifest, missingIdentity).errors.join("\n"),
    /identity map is missing claude_code/,
  );

  const duplicateIdentity = structuredClone(claudeManifest);
  duplicateIdentity.worker.principal = "claude_cli";
  assert.match(
    validateManifest(duplicateIdentity, identityMap).errors.join("\n"),
    /must bind to claude_code/,
  );

  const modeDrift = structuredClone(claudeManifest);
  modeDrift.posture.permissionMode = "default";
  assert.match(
    validateManifest(modeDrift, identityMap).errors.join("\n"),
    /must be bypass-permissions/,
  );
  const missingPromptPin = structuredClone(cursorManifest);
  delete missingPromptPin.runtime.systemPromptSha256;
  assert.match(
    validateManifest(missingPromptPin, identityMap).errors.join("\n"),
    /systemPromptSha256/,
  );

  const adapterDrift = structuredClone(claudeManifest);
  adapterDrift.runtime.claudeAcp.integrity = "sha512-ZHJpZnQ=";
  assert.match(
    validateManifest(adapterDrift, identityMap).errors.join("\n"),
    /package integrity drift/,
  );

  const closureDrift = structuredClone(claudeManifest);
  closureDrift.runtime.claudeAcp.closureSha256 = "0".repeat(64);
  assert.match(
    validateManifest(closureDrift, identityMap).errors.join("\n"),
    /package closure checkpoint drift/,
  );

  const configRelocation = structuredClone(claudeManifest);
  configRelocation.runtime.claudeCode.configDir = "/Users/architect/.claude";
  assert.match(
    validateManifest(configRelocation, identityMap).errors.join("\n"),
    /config directory override must be absent/,
  );

  const volumeRuntime = structuredClone(claudeManifest);
  volumeRuntime.runtime.buzzAcpBinary = "/Volumes/AEON/runtime/buzz-acp";
  assert.match(
    validateManifest(volumeRuntime, identityMap).errors.join("\n"),
    /canonical Data-volume path/,
  );

  const sharedHarnessDrift = structuredClone(claudeManifest);
  sharedHarnessDrift.runtime.buzzAcpSha256 = "0".repeat(64);
  assert.match(
    validateManifest(sharedHarnessDrift, identityMap).errors.join("\n"),
    /shared buzz-acp checkpoint drift/,
  );

  const signerDrift = structuredClone(claudeManifest);
  signerDrift.runtime.signerPath = identityMap.members.claude_code.secret_ref;
  assert.match(
    validateManifest(signerDrift, identityMap).errors.join("\n"),
    /launchd-safe Data-volume path/,
  );

  const nodeDrift = structuredClone(claudeManifest);
  nodeDrift.runtime.node.sha256 = "0".repeat(64);
  assert.match(
    validateManifest(nodeDrift, identityMap).errors.join("\n"),
    /Node runtime checkpoint drift/,
  );

  const nodePathFallback = structuredClone(claudeManifest);
  nodePathFallback.runtime.path.reverse();
  assert.match(
    validateManifest(nodePathFallback, identityMap).errors.join("\n"),
    /Node runtime first/,
  );
});

test("Cursor contract rejects identity, runtime, auth, and model drift", () => {
  const missingIdentity = structuredClone(identityMap);
  delete missingIdentity.members.cursor_cli;
  assert.match(
    validateManifest(cursorManifest, missingIdentity).errors.join("\n"),
    /identity map is missing cursor_cli/,
  );

  const modeDrift = structuredClone(cursorManifest);
  modeDrift.posture.permissionMode = "default";
  assert.match(
    validateManifest(modeDrift, identityMap).errors.join("\n"),
    /must be bypass-permissions/,
  );

  for (const mutate of [
    (value) => (value.runtime.cursorAcp.version = "future"),
    (value) => (value.runtime.cursorAcp.entrypointSha256 = "0".repeat(64)),
    (value) => (value.runtime.cursorAcp.closureSha256 = "0".repeat(64)),
    (value) => (value.runtime.cursorAcp.args = ["acp"]),
    (value) => (value.runtime.cursorAcp.auth.subscriptionTypes = ["Free"]),
    (value) => (value.runtime.cursorAcp.model.requested = "cursor-grok-4.5-high-fast"),
    (value) => (value.runtime.cursorAcp.model.effective = "cursor-grok-4.5-high-fast"),
    (value) => (value.runtime.signerPath = identityMap.members.cursor_cli.secret_ref),
    (value) => (value.runtime.bootstrapPath = "/tmp/cursor-acp-bootstrap.cjs"),
    (value) => (value.runtime.bootstrapSha256 = "0".repeat(64)),
  ]) {
    const drift = structuredClone(cursorManifest);
    mutate(drift);
    assert.match(validateManifest(drift, identityMap).errors.join("\n"), /Cursor/);
  }
});

test("Grok contract rejects identity, runtime, auth, and model drift", () => {
  const missingIdentity = structuredClone(identityMap);
  delete missingIdentity.members.grok_cli;
  assert.match(
    validateManifest(grokManifest, missingIdentity).errors.join("\n"),
    /identity map is missing grok_cli/,
  );

  for (const mutate of [
    (value) => (value.runtime.grokAcp.version = "future"),
    (value) => (value.runtime.grokAcp.entrypointSha256 = "0".repeat(64)),
    (value) => (value.runtime.grokAcp.args = ["agent", "stdio"]),
    (value) => (value.runtime.grokAcp.auth.provider = "api-key"),
    (value) => (value.runtime.grokAcp.model.effective = "grok-4-fast"),
    (value) => (value.runtime.signerPath = identityMap.members.grok_cli.secret_ref),
  ]) {
    const drift = structuredClone(grokManifest);
    mutate(drift);
    assert.match(validateManifest(drift, identityMap).errors.join("\n"), /Grok/);
  }
});

test("Grok worker rejects ambient API and auth overrides", () => {
  const clean = validateAmbientGrokOverrides({});
  assert.equal(clean.ok, true);
  const dirty = validateAmbientGrokOverrides({
    XAI_API_KEY: "sentinel",
    GROK_HOME: "/tmp/other",
  });
  assert.equal(dirty.ok, false);
  assert.deepEqual(dirty.errors, [
    "XAI_API_KEY must be absent for Grok subscription authentication",
    "GROK_HOME must be absent for Grok subscription authentication",
  ]);
});

test("Codex rejects shared harness path and digest drift under the safe supervisor", () => {
  const pathDrift = structuredClone(manifest);
  pathDrift.runtime.buzzAcpBinary = "/Volumes/AEON/runtime/buzz-acp";
  assert.match(
    validateManifest(pathDrift, identityMap).errors.join("\n"),
    /canonical Data-volume path/,
  );

  const digestDrift = structuredClone(manifest);
  digestDrift.runtime.buzzAcpSha256 = "0".repeat(64);
  assert.match(
    validateManifest(digestDrift, identityMap).errors.join("\n"),
    /shared buzz-acp checkpoint drift/,
  );

  const artifact = renderDisabledLaunchAgent(manifest, identityMap, "codex");
  assert.equal(
    artifact.workingDirectory,
    "/Users/architect/Library/Application Support/AEON/aeon-v6",
  );
  assert.equal(artifact.sessionCwd, "/Volumes/AEON/Projects/codex");
  assert.equal(
    artifact.args[artifact.args.indexOf("--session-cwd") + 1],
    "/Volumes/AEON/Projects/codex",
  );
});

test("Claude package closure digest detects adapter and dependency changes", () => {
  const root = mkdtempSync(join(tmpdir(), "claude-agent-acp-closure-"));
  try {
    mkdirSync(join(root, "dist"), { recursive: true });
    writeFileSync(join(root, "dist", "index.js"), "entrypoint\n");
    const sibling = join(root, "dist", "acp-agent.js");
    writeFileSync(sibling, "original sibling\n");
    mkdirSync(join(root, "node_modules", "dependency"), { recursive: true });
    const dependency = join(root, "node_modules", "dependency", "index.js");
    writeFileSync(dependency, "original dependency\n");

    const initial = hashPackageClosure(root);
    assert.equal(hashPackageClosure(root), initial);
    writeFileSync(sibling, "modified sibling\n");
    const siblingChanged = hashPackageClosure(root);
    assert.notEqual(siblingChanged, initial);
    writeFileSync(sibling, "original sibling\n");
    writeFileSync(dependency, "modified dependency\n");
    assert.notEqual(hashPackageClosure(root), initial);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("Cursor closure digest excludes only transient running markers", () => {
  const root = mkdtempSync(join(tmpdir(), "cursor-agent-closure-"));
  try {
    writeFileSync(join(root, "index.js"), "entrypoint\n");
    mkdirSync(join(root, ".running"));
    writeFileSync(join(root, ".running", "first"), "pid\n");
    const initial = hashCursorClosure(root);
    writeFileSync(join(root, ".running", "second"), "different pid\n");
    assert.equal(hashCursorClosure(root), initial);
    writeFileSync(join(root, "index.js"), "changed\n");
    assert.notEqual(hashCursorClosure(root), initial);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("Claude runtime auth requires the pinned subscription provider and type", () => {
  const contract = claudeManifest.runtime.claudeCode.auth;
  const valid = {
    loggedIn: true,
    authMethod: "claude.ai",
    apiProvider: "firstParty",
    subscriptionType: "pro",
  };
  assert.deepEqual(validateClaudeSubscriptionAuth(valid, contract), {
    ok: true,
    errors: [],
  });

  const wrongMethod = validateClaudeSubscriptionAuth({ ...valid, authMethod: "apiKey" }, contract);
  assert.equal(wrongMethod.ok, false);
  assert.match(wrongMethod.errors.join("\n"), /auth method/);

  const wrongProvider = validateClaudeSubscriptionAuth(
    { ...valid, apiProvider: "bedrock" },
    contract,
  );
  assert.equal(wrongProvider.ok, false);
  assert.match(wrongProvider.errors.join("\n"), /API provider/);

  const wrongSubscription = validateClaudeSubscriptionAuth(
    { ...valid, subscriptionType: "free" },
    contract,
  );
  assert.equal(wrongSubscription.ok, false);
  assert.match(wrongSubscription.errors.join("\n"), /subscription type/);
});

test("Claude runtime rejects ambient API credentials without exposing values", () => {
  assert.deepEqual(validateAmbientAnthropicCredentials({}), {
    ok: true,
    errors: [],
  });
  const result = validateAmbientAnthropicCredentials({
    ANTHROPIC_API_KEY: "secret-api-key",
    ANTHROPIC_AUTH_TOKEN: "secret-auth-token",
  });
  assert.equal(result.ok, false);
  assert.deepEqual(result.errors, [
    "ANTHROPIC_API_KEY must be absent for Claude subscription authentication",
    "ANTHROPIC_AUTH_TOKEN must be absent for Claude subscription authentication",
  ]);
  assert.doesNotMatch(result.errors.join("\n"), /secret-/);
});

test("Cursor runtime requires the pinned subscription without exposing account data", () => {
  const contract = cursorManifest.runtime.cursorAcp.auth;
  const validStatus = { status: "authenticated", isAuthenticated: true };
  const validAbout = { subscriptionTier: "Pro" };
  assert.deepEqual(validateCursorSubscriptionAuth(validStatus, validAbout, contract), {
    ok: true,
    errors: [],
  });
  assert.match(
    validateCursorSubscriptionAuth(
      { ...validStatus, isAuthenticated: false },
      validAbout,
      contract,
    ).errors.join("\n"),
    /unavailable/,
  );
  assert.match(
    validateCursorSubscriptionAuth(validStatus, { subscriptionTier: "Free" }, contract).errors.join(
      "\n",
    ),
    /subscription type/,
  );
});

test("Cursor worker scrubs API and endpoint overrides", () => {
  assert.deepEqual(validateAmbientCursorOverrides({}), {
    ok: true,
    errors: [],
  });
  const result = validateAmbientCursorOverrides({
    CURSOR_API_KEY: "secret-key",
    CURSOR_API_ENDPOINT: "https://example.invalid",
  });
  assert.equal(result.ok, false);
  assert.deepEqual(result.errors, [
    "CURSOR_API_KEY must be absent for Cursor subscription authentication",
    "CURSOR_API_ENDPOINT must be absent for Cursor subscription authentication",
  ]);
  assert.doesNotMatch(result.errors.join("\n"), /secret-key|example\.invalid/);
});

test("Claude Node validation rejects mode, symlink, hash, and version drift", () => {
  const root = mkdtempSync(join(tmpdir(), "claude-node-runtime-"));
  try {
    const binary = join(root, "node");
    writeFileSync(binary, "#!/bin/sh\nprintf 'v-test\\n'\n");
    chmodSync(binary, 0o500);
    const sha256 = createHash("sha256").update("#!/bin/sh\nprintf 'v-test\\n'\n").digest("hex");
    const pin = { binary, mode: "0500", sha256, version: "v-test" };
    assert.deepEqual(validatePinnedNodeRuntime(pin, process.env), {
      ok: true,
      errors: [],
    });

    chmodSync(binary, 0o400);
    const badMode = validatePinnedNodeRuntime(pin, process.env);
    assert.equal(badMode.ok, false);
    assert.match(badMode.errors.join("\n"), /mode must be 0500/);
    chmodSync(binary, 0o500);

    const marker = join(root, "executed");
    chmodSync(binary, 0o700);
    writeFileSync(binary, `#!/bin/sh\ntouch '${marker}'\nprintf 'v-test\\n'\n`);
    chmodSync(binary, 0o500);
    const badHash = validatePinnedNodeRuntime({ ...pin, sha256: "0".repeat(64) }, process.env);
    assert.match(badHash.errors.join("\n"), /SHA-256/);
    assert.equal(existsSync(marker), false);
    chmodSync(binary, 0o700);
    writeFileSync(binary, "#!/bin/sh\nprintf 'v-test\\n'\n");
    chmodSync(binary, 0o500);
    assert.match(
      validatePinnedNodeRuntime({ ...pin, version: "v-wrong" }, process.env).errors.join("\n"),
      /version/,
    );

    const link = join(root, "node-link");
    symlinkSync(binary, link);
    assert.match(
      validatePinnedNodeRuntime({ ...pin, binary: link }, process.env).errors.join("\n"),
      /non-symlink/,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("verified receipt joins request, session, run, and signed reply", () => {
  const requestEventId = "a".repeat(64);
  const replyEventId = "b".repeat(64);
  const channelId = identityMap.channels.ops.channel_id;
  const result = correlateVerifiedReceipt({
    requestEventId,
    channelId,
    expectedPubkey: identityMap.members.codex_cli.pubkey_hex,
    replyEvent: {
      id: replyEventId,
      pubkey: identityMap.members.codex_cli.pubkey_hex,
      kind: 9,
      verified: true,
      tags: [
        ["h", channelId],
        ["e", requestEventId, "", "reply"],
      ],
    },
    observerRun: {
      requestEventId,
      replyEventId,
      channelId,
      sessionId: "codex-acp-session",
      runId: "buzz-turn-id",
    },
  });
  assert.deepEqual(result, {
    requestEventId,
    replyEventId,
    sessionId: "codex-acp-session",
    runId: "buzz-turn-id",
    channelId,
  });
});

test("receipt correlation rejects unsigned or mismatched replies", () => {
  const requestEventId = "a".repeat(64);
  const channelId = identityMap.channels.ops.channel_id;
  const base = {
    requestEventId,
    channelId,
    expectedPubkey: identityMap.members.codex_cli.pubkey_hex,
    replyEvent: {
      id: "b".repeat(64),
      pubkey: identityMap.members.codex_cli.pubkey_hex,
      kind: 9,
      verified: false,
      tags: [
        ["h", channelId],
        ["e", requestEventId, "", "reply"],
      ],
    },
    observerRun: {
      requestEventId,
      replyEventId: "b".repeat(64),
      channelId,
      sessionId: "session",
      runId: "run",
    },
  };
  assert.throws(() => correlateVerifiedReceipt(base), /signature/);
  base.replyEvent.verified = true;
  base.observerRun.replyEventId = "c".repeat(64);
  assert.throws(() => correlateVerifiedReceipt(base), /correlation/);
});
