#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  hashPackageClosure,
  hashCursorClosure,
  loadJson,
  renderDisabledLaunchAgent,
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
function option(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

const optionValues = new Set([option("--worker")].filter(Boolean));
const positional = process.argv
  .slice(2)
  .filter((arg) => !arg.startsWith("--") && !optionValues.has(arg));
const identityPath = positional[0] ?? join(here, "fixtures", "identity-map.json");
const worker = option("--worker") ?? "codex_cli";
if (!["codex_cli", "claude_cli", "cursor_cli", "grok_cli"].includes(worker)) {
  console.error(`unsupported external CLI worker: ${worker}`);
  process.exit(1);
}
const manifestName = worker === "codex_cli" ? "manifest.json" : `manifest.${worker}.json`;
const manifest = loadJson(join(here, manifestName));
const identityMap = loadJson(identityPath);
const validation = validateManifest(manifest, identityMap);
if (!validation.ok) {
  console.error(validation.errors.join("\n"));
  process.exit(1);
}

function validatePinnedPrompt(runtime) {
  if (!runtime.systemPromptPath) return;
  const promptStat = fs.lstatSync(runtime.systemPromptPath);
  if (
    !promptStat.isFile() ||
    promptStat.isSymbolicLink() ||
    (promptStat.mode & 0o777) !== 0o444
  ) {
    throw new Error("system prompt must be a regular mode-0444 file");
  }
  const promptSha256 = createHash("sha256")
    .update(fs.readFileSync(runtime.systemPromptPath))
    .digest("hex");
  if (promptSha256 !== runtime.systemPromptSha256) {
    throw new Error("system prompt SHA-256 does not match the manifest pin");
  }
}

const selector = manifest.worker.selector ?? manifest.worker.principal;
const configText = fs.readFileSync(join(here, "config", `${selector}.toml`), "utf8");
const subscriptionValidation = validateSubscriptionProjection(configText, manifest, identityMap);
if (!subscriptionValidation.ok) {
  console.error(subscriptionValidation.errors.join("\n"));
  process.exit(1);
}

const artifact = renderDisabledLaunchAgent(manifest, identityMap);
if (artifact.plist.includes("BUZZ_PRIVATE_KEY") || artifact.plist.includes("nsec1")) {
  console.error("rendered artifact contains signer material");
  process.exit(1);
}
if (artifact.args.includes("--no-agent-publisher-credentials")) {
  console.error(
    `external ${manifest.worker.principal} must receive its own managed Buzz credentials`,
  );
  process.exit(1);
}
if (!artifact.args.includes("--agent-publisher-credentials")) {
  console.error(
    `external ${manifest.worker.principal} must explicitly opt into managed Buzz credentials`,
  );
  process.exit(1);
}
if (
  artifact.args.filter((arg) => arg === "--agent-publisher-credentials").length !== 1 ||
  artifact.args[artifact.args.indexOf("--subscribe") + 1] !== manifest.posture.subscribe ||
  artifact.args[artifact.args.indexOf("--config") + 1] !== manifest.runtime.configPath ||
  artifact.args[artifact.args.indexOf("--expected-public-key") + 1] !==
    identityMap.members[manifest.worker.principal].pubkey_hex ||
  artifact.subscriptionRoomIds.join("\n") !== subscriptionValidation.roomIds.join("\n")
) {
  console.error("rendered launch argv does not match the source projection");
  process.exit(1);
}

const runtimeCheck = process.argv.includes("--runtime");
if (runtimeCheck) {
  const adapter =
    selector === "codex_cli"
      ? manifest.runtime.codexAcp
      : selector === "claude_cli"
        ? manifest.runtime.claudeAcp
        : selector === "cursor_cli"
          ? manifest.runtime.cursorAcp
          : manifest.runtime.grokAcp;
  fs.accessSync(manifest.runtime.buzzAcpBinary, fs.constants.X_OK);
  const buzzSha256 = createHash("sha256")
    .update(fs.readFileSync(manifest.runtime.buzzAcpBinary))
    .digest("hex");
  if (buzzSha256 !== manifest.runtime.buzzAcpSha256) {
    throw new Error("shared buzz-acp SHA-256 does not match the manifest pin");
  }
  const buzzHelp = spawnSync(manifest.runtime.buzzAcpBinary, ["--help"], {
    encoding: "utf8",
    env: artifact.environment,
  });
  if (
    buzzHelp.status !== 0 ||
    !buzzHelp.stdout.includes("--agent-publisher-credentials") ||
    !buzzHelp.stdout.includes("--no-agent-publisher-credentials") ||
    !buzzHelp.stdout.includes("--session-cwd") ||
    !buzzHelp.stdout.includes("strict-allowlist")
  ) {
    throw new Error("shared buzz-acp does not advertise the required external worker contract");
  }
  fs.accessSync(adapter.binary, fs.constants.X_OK);
  for (const directory of artifact.requiredDirectories) {
    if (!fs.statSync(directory).isDirectory()) {
      throw new Error(`required runtime path is not a directory: ${directory}`);
    }
  }
  validatePinnedPrompt(manifest.runtime);
  if (selector === "claude_cli") {
    const nodeValidation = validatePinnedNodeRuntime(manifest.runtime.node, artifact.environment);
    if (!nodeValidation.ok) throw new Error(nodeValidation.errors.join("\n"));
  } else if (selector === "codex_cli") {
    const nodeBinary = manifest.runtime.path
      .map((directory) => join(directory, "node"))
      .find((candidate) => {
        try {
          fs.accessSync(candidate, fs.constants.X_OK);
          return true;
        } catch {
          return false;
        }
      });
    if (!nodeBinary) {
      throw new Error("rendered PATH does not contain an executable Node runtime");
    }
  }
  const adapterEntrypoint = fs.realpathSync(adapter.binary);
  const adapterSha256 = createHash("sha256")
    .update(fs.readFileSync(adapterEntrypoint))
    .digest("hex");
  if (adapterSha256 !== adapter.entrypointSha256) {
    throw new Error(
      `${manifest.worker.principal} ACP entrypoint SHA-256 does not match the manifest pin`,
    );
  }
  const signerProbe = spawnSync(
    manifest.runtime.buzzAcpBinary,
    [
      "--private-key-file",
      artifact.signerFile,
      "--expected-public-key",
      artifact.expectedPublicKey,
      "--heartbeat-interval",
      "1",
    ],
    {
      encoding: "utf8",
      env: artifact.environment,
    },
  );
  if (
    signerProbe.status === 0 ||
    !signerProbe.stderr.includes("heartbeat interval must be 0 (disabled)")
  ) {
    throw new Error(`shared buzz-acp signer validation failed: ${signerProbe.stderr.trim()}`);
  }
  if (selector === "codex_cli") {
    fs.accessSync(manifest.runtime.codexHome, fs.constants.R_OK);
    const adapterVersion = spawnSync(adapter.binary, ["--version"], {
      encoding: "utf8",
      env: artifact.environment,
    });
    if (adapterVersion.status !== 0) {
      throw new Error(`codex-acp --version failed: ${adapterVersion.stderr.trim()}`);
    }
    if (!adapterVersion.stdout.includes(` ${adapter.version}`)) {
      throw new Error(`codex-acp version does not match ${adapter.version}`);
    }
  } else if (selector === "claude_cli") {
    const packageRoot = dirname(dirname(adapterEntrypoint));
    const packageJson = loadJson(join(packageRoot, "package.json"));
    if (packageJson.name !== adapter.package || packageJson.version !== adapter.version) {
      throw new Error("claude-agent-acp package metadata does not match the manifest pin");
    }
    if (hashPackageClosure(adapter.root) !== adapter.closureSha256) {
      throw new Error("claude-agent-acp installed package closure does not match the manifest pin");
    }
    const adapterVersion = spawnSync(adapter.binary, ["--version"], {
      encoding: "utf8",
      env: artifact.environment,
    });
    if (adapterVersion.status !== 0 || adapterVersion.stdout.trim() !== adapter.version) {
      throw new Error(`claude-agent-acp version does not match ${adapter.version}`);
    }
    fs.accessSync(manifest.runtime.claudeCode.binary, fs.constants.X_OK);
    const claudeSha256 = createHash("sha256")
      .update(fs.readFileSync(manifest.runtime.claudeCode.binary))
      .digest("hex");
    if (claudeSha256 !== manifest.runtime.claudeCode.binarySha256) {
      throw new Error("Claude Code binary SHA-256 does not match the manifest pin");
    }
    const ambientCredentials = validateAmbientAnthropicCredentials(process.env);
    if (!ambientCredentials.ok) {
      throw new Error(ambientCredentials.errors.join("\n"));
    }
    const standardClaudeEnvironment = {
      ...process.env,
      ...artifact.environment,
    };
    delete standardClaudeEnvironment.CLAUDE_CONFIG_DIR;
    delete standardClaudeEnvironment.ANTHROPIC_API_KEY;
    delete standardClaudeEnvironment.ANTHROPIC_AUTH_TOKEN;
    const claudeVersion = spawnSync(manifest.runtime.claudeCode.binary, ["--version"], {
      encoding: "utf8",
      env: standardClaudeEnvironment,
    });
    if (
      claudeVersion.status !== 0 ||
      !claudeVersion.stdout.startsWith(manifest.runtime.claudeCode.version)
    ) {
      throw new Error(`Claude Code version does not match ${manifest.runtime.claudeCode.version}`);
    }
    const authStatus = spawnSync(manifest.runtime.claudeCode.binary, ["auth", "status"], {
      encoding: "utf8",
      env: standardClaudeEnvironment,
    });
    let auth;
    try {
      auth = JSON.parse(authStatus.stdout);
    } catch {
      throw new Error("Claude Code auth status did not return JSON");
    }
    if (authStatus.status !== 0) {
      throw new Error("Claude Code auth status failed");
    }
    const authValidation = validateClaudeSubscriptionAuth(auth, manifest.runtime.claudeCode.auth);
    if (!authValidation.ok) {
      throw new Error(authValidation.errors.join("\n"));
    }
  } else if (selector === "cursor_cli") {
    const bootstrapStat = fs.lstatSync(manifest.runtime.bootstrapPath);
    if (
      !bootstrapStat.isFile() ||
      bootstrapStat.isSymbolicLink() ||
      (bootstrapStat.mode & 0o777) !== 0o444
    ) {
      throw new Error("Cursor ACP bootstrap must be a regular mode-0444 file");
    }
    const bootstrapSha256 = createHash("sha256")
      .update(fs.readFileSync(manifest.runtime.bootstrapPath))
      .digest("hex");
    if (bootstrapSha256 !== manifest.runtime.bootstrapSha256) {
      throw new Error("Cursor ACP bootstrap SHA-256 does not match the manifest pin");
    }
    if (hashCursorClosure(adapter.root) !== adapter.closureSha256) {
      throw new Error("Cursor CLI installed closure does not match the manifest pin");
    }
    const ambientOverrides = validateAmbientCursorOverrides(process.env);
    if (!ambientOverrides.ok) throw new Error(ambientOverrides.errors.join("\n"));
    const cursorEnvironment = { ...process.env, ...artifact.environment };
    delete cursorEnvironment.CURSOR_API_KEY;
    delete cursorEnvironment.CURSOR_API_ENDPOINT;
    const cursorVersion = spawnSync(adapter.binary, ["--version"], {
      encoding: "utf8",
      env: cursorEnvironment,
    });
    if (cursorVersion.status !== 0 || cursorVersion.stdout.trim() !== adapter.version) {
      throw new Error(`Cursor CLI version does not match ${adapter.version}`);
    }
    const status = spawnSync(adapter.binary, ["status", "--format", "json"], {
      encoding: "utf8",
      env: cursorEnvironment,
    });
    const about = spawnSync(adapter.binary, ["about", "--format", "json"], {
      encoding: "utf8",
      env: cursorEnvironment,
    });
    let statusJson;
    let aboutJson;
    try {
      statusJson = JSON.parse(status.stdout);
      aboutJson = JSON.parse(about.stdout);
    } catch {
      throw new Error("Cursor auth status did not return JSON");
    }
    if (status.status !== 0 || about.status !== 0) {
      throw new Error("Cursor auth status failed");
    }
    const authValidation = validateCursorSubscriptionAuth(statusJson, aboutJson, adapter.auth);
    if (!authValidation.ok) throw new Error(authValidation.errors.join("\n"));
    const modelCatalog = spawnSync(adapter.binary, ["models"], {
      encoding: "utf8",
      env: cursorEnvironment,
    });
    if (modelCatalog.status !== 0) {
      throw new Error("Cursor model catalog query failed");
    }
    const catalogModelIds = new Set(
      modelCatalog.stdout
        .split("\n")
        .map((line) => line.match(/^(\S+) - /)?.[1])
        .filter(Boolean),
    );
    if (!catalogModelIds.has(adapter.model.requested)) {
      throw new Error("Cursor requested model alias is absent from its native catalog");
    }
    const acpModelArgs = [
      "models",
      "--agent-command",
      `${adapter.root}/node`,
      `--agent-args=${[
        "--use-system-ca",
        manifest.runtime.bootstrapPath,
        manifest.workspaces.allowed[manifest.workspaces.default],
        `${adapter.root}/index.js`,
        ...adapter.args,
      ].join(",")}`,
      "--json",
    ];
    const acpModelCatalog = spawnSync(manifest.runtime.buzzAcpBinary, acpModelArgs, {
      cwd: manifest.runtime.supervisorWorkingDirectory,
      encoding: "utf8",
      env: cursorEnvironment,
    });
    let acpCatalog;
    try {
      acpCatalog = JSON.parse(acpModelCatalog.stdout);
    } catch {
      throw new Error("Cursor ACP model catalog did not return JSON");
    }
    const acpModelIds = new Set(
      (acpCatalog?.unstable?.availableModels ?? []).map((model) => model.modelId),
    );
    if (acpModelCatalog.status !== 0 || !acpModelIds.has(adapter.model.effective)) {
      throw new Error("Cursor effective ACP model is absent from its catalog");
    }
  } else {
    const authStat = fs.lstatSync(adapter.auth.authFile);
    if (!authStat.isFile() || authStat.isSymbolicLink() || (authStat.mode & 0o777) !== 0o600) {
      throw new Error("Grok auth file must be a regular non-symlink file with mode 0600");
    }
    if (fs.realpathSync(adapter.binary) !== adapter.realBinary) {
      throw new Error("Grok real binary does not match the manifest pin");
    }
    const ambientOverrides = validateAmbientGrokOverrides(process.env);
    if (!ambientOverrides.ok) {
      throw new Error(ambientOverrides.errors.join("\n"));
    }
    const grokEnvironment = { ...process.env, ...artifact.environment };
    for (const name of [
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
    ]) {
      delete grokEnvironment[name];
    }
    const grokVersion = spawnSync(adapter.binary, ["--version"], {
      encoding: "utf8",
      env: grokEnvironment,
    });
    if (
      grokVersion.status !== 0 ||
      !grokVersion.stdout.startsWith(`grok ${adapter.version} (${adapter.build})`)
    ) {
      throw new Error(`Grok version does not match ${adapter.version} (${adapter.build})`);
    }
    const grokModels = spawnSync(adapter.binary, ["models"], {
      encoding: "utf8",
      env: grokEnvironment,
    });
    if (
      grokModels.status !== 0 ||
      !grokModels.stdout.includes("You are logged in with grok.com.") ||
      !grokModels.stdout.includes(`Default model: ${adapter.model.effective}`)
    ) {
      throw new Error("Grok existing login or model checkpoint is unavailable");
    }
    const modelArgs = [
      "models",
      "--agent-command",
      adapter.binary,
      "--agent-args",
      adapter.args.join(","),
      "--json",
    ];
    const modelCatalog = spawnSync(manifest.runtime.buzzAcpBinary, modelArgs, {
      encoding: "utf8",
      env: grokEnvironment,
    });
    let catalog;
    try {
      catalog = JSON.parse(modelCatalog.stdout);
    } catch {
      throw new Error("Grok ACP model catalog did not return JSON");
    }
    const available = catalog?.unstable?.availableModels ?? [];
    const selected = available.find((model) => model.modelId === adapter.model.effective);
    if (
      modelCatalog.status !== 0 ||
      catalog?.unstable?.currentModelId !== adapter.model.effective ||
      selected?._meta?.reasoningEffort !== adapter.model.reasoningEffort
    ) {
      throw new Error("Grok ACP model or reasoning checkpoint drift");
    }
  }
}

const result = {
  ok: true,
  enabled: false,
  principal: manifest.worker.principal,
  ...(selector !== manifest.worker.principal ? { worker: selector } : {}),
  workspace: artifact.sessionCwd,
  ...(selector === "codex_cli"
    ? { agentMode: artifact.environment.INITIAL_AGENT_MODE }
    : { permissionMode: manifest.posture.permissionMode }),
  roomCount: subscriptionValidation.roomIds.length,
  publisherCredentials: "managed",
  ...(selector === "cursor_cli"
    ? {
        requestedModel: manifest.runtime.cursorAcp.model.requested,
        effectiveModel: manifest.runtime.cursorAcp.model.effective,
        modelSelectionStatus: manifest.runtime.cursorAcp.model.selectionStatus,
      }
    : {}),
  ...(selector === "grok_cli"
    ? {
        requestedModel: manifest.runtime.grokAcp.model.requested,
        effectiveModel: manifest.runtime.grokAcp.model.effective,
        reasoningEffort: manifest.runtime.grokAcp.model.reasoningEffort,
      }
    : {}),
  runtimeCheck,
};
process.stdout.write(`${JSON.stringify(result)}\n`);
