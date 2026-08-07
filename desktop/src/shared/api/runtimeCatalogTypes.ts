/**
 * The ACP runtime catalog entry the UI consumes, split from `types.ts` to keep
 * that file under the repo's file-size ratchet.
 */

import type { AcpAvailabilityStatus, AuthStatus } from "./types";

export type AcpRuntimeCatalogEntry = {
  id: string;
  label: string;
  avatarUrl: string;
  availability: AcpAvailabilityStatus;
  command: string | null;
  binaryPath: string | null;
  defaultArgs: string[];
  mcpCommand: string | null;
  /** Environment variable used to apply the initial model, when supported. */
  modelEnvVar: string | null;
  /** Environment variable used to apply the selected LLM provider, when supported. */
  providerEnvVar: string | null;
  /** Environment variable used to apply thinking effort, when supported. */
  thinkingEnvVar: string | null;
  /** Structured JSON env-var target for thinking effort, when supported. */
  thinkingConfigJsonEnvVar: string | null;
  /** JSON property in thinkingConfigJsonEnvVar holding the effort value. */
  thinkingConfigJsonKey: string | null;
  maxTokensEnvVar: string | null;
  contextLimitEnvVar: string | null;
  maxRoundsEnvVar: string | null;
  installHint: string;
  installInstructionsUrl: string;
  canAutoInstall: boolean;
  /** True when the runtime depends on a separately installed vendor CLI. */
  requiresExternalCli: boolean;
  underlyingCliPath: string | null;
  /** True when an npm adapter step is pending but Node.js / npm is absent. */
  nodeRequired: boolean;
  /** Login/auth status for CLI-based runtimes. */
  authStatus: AuthStatus;
  /** Hint for completing authentication; null when not applicable or already logged in. */
  loginHint: string | null;
  /** "builtin" (compiled in), "preset" (PATH-probed, not editable), or "custom" (user JSON). Controls UI editability. */
  source: "builtin" | "preset" | "custom";
  /**
   * Definition-level env vars for `source: custom` entries. Populated from
   * `HarnessDefinition.env` so saves don't erase existing vars. Absent for
   * builtin/preset entries.
   */
  definitionEnv?: Record<string, string>;
  /** Spawn-time parallelism cap; absent for uncapped harnesses. */
  maxParallelism?: number;
};

/** An AcpRuntimeCatalogEntry that is confirmed available — command and binaryPath are non-null. */
export type AcpRuntime = AcpRuntimeCatalogEntry & {
  availability: "available";
  command: string;
  binaryPath: string;
};
