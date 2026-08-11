import type { RestartDiffEntry } from "./restartDiff";

export type ManagedAgentBackend =
  | { type: "local" }
  | { type: "provider"; id: string; config: Record<string, unknown> };

/** Durable owner designation that makes source preflight mandatory. */
export type HeartbeatPreflightDesignation = {
  policyFile: string;
  policySha256: string;
  heartbeatIntervalSeconds: number;
};

/** Inbound author gate mode. Mirrors buzz-acp's --respond-to CLI flag. */
export type RespondToMode = "owner-only" | "allowlist" | "anyone";

export type ManagedAgent = {
  pubkey: string;
  name: string;
  personaId: string | null;
  /**
   * The record's harness/runtime id (e.g. "goose", "my-custom-harness").
   * `null` means the agent inherits its harness from the linked persona.
   * Used to count agents referencing a harness definition (delete confirm).
   */
  runtime: string | null;
  teamId?: string | null;
  relayUrl: string;
  acpCommand: string;
  /** Resolved/effective harness command (persona-wins, override-honored). */
  agentCommand: string;
  /**
   * Explicit per-instance harness pin. `null` means the agent inherits its
   * harness from the linked persona's runtime. Lets the Edit dialog show
   * "Inherit from persona" vs a concrete pin.
   */
  agentCommandOverride: string | null;
  agentArgs: string[];
  mcpCommand: string;
  turnTimeoutSeconds: number;
  idleTimeoutSeconds: number | null;
  maxTurnDurationSeconds: number | null;
  parallelism: number;
  systemPrompt: string | null;
  avatarUrl: string | null;
  model: string | null;
  modelSource: "definition" | "global" | "instance_legacy" | null;
  /** LLM inference provider, from the agent's pinned record snapshot. */
  provider: string | null;
  /**
   * `true` when the linked persona has been edited since this agent was
   * created — the running agent uses the older pinned snapshot. Surface a
   * "out of date" marker and prompt the user to delete + respawn to update.
   * Always `false` for non-persona agents and for orphaned agents.
   */
  personaOutOfDate: boolean;
  /**
   * `true` when the agent's linked persona no longer exists. Distinct from
   * out-of-date: there is no current persona to respawn into, so do not prompt
   * a respawn — the pinned snapshot is all the config that remains.
   */
  personaOrphaned: boolean;
  /**
   * `true` when the running process was spawned with a config that no longer
   * matches what a spawn would use today — a plain restart would change what
   * runs. Complements `personaOutOfDate` ("a respawn would change it").
   * Always `false` for stopped agents.
   */
  needsRestart: boolean;
  /** Non-empty iff `needsRestart` is true. Empty when Rust omits the field. */
  restartDiff: RestartDiffEntry[];
  /** Per-agent env vars. Layered on top of persona envVars. */
  envVars: Record<string, string>;
  status: "running" | "stopped" | "deployed" | "not_deployed";
  pid: number | null;
  createdAt: string;
  updatedAt: string;
  lastStartedAt: string | null;
  lastStoppedAt: string | null;
  lastExitCode: number | null;
  lastError: string | null;
  lastErrorCode: number | null;
  logPath: string;
  startOnAppLaunch: boolean;
  autoRestartOnConfigChange: boolean;
  backend: ManagedAgentBackend;
  backendAgentId: string | null;
  /** Who the agent should respond to. Maps to `buzz-acp --respond-to`. */
  respondTo: RespondToMode;
  /**
   * Normalized 64-char lowercase hex pubkeys. Used only when `respondTo` is
   * `"allowlist"`. Preserved across mode toggles.
   */
  respondToAllowlist: string[];
  /** Present only when the owner has durably designated this agent must-check. */
  heartbeatPreflight?: HeartbeatPreflightDesignation | null;
};
