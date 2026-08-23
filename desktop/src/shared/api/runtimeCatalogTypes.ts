import type { AcpProviderProfile } from "./providerProfiles";

export type AcpAvailabilityStatus =
  | "available"
  | "adapter_missing"
  | "adapter_outdated"
  | "cli_missing"
  | "not_installed";

/** Authentication/login status for a CLI-based ACP runtime. */
export type AuthStatus =
  | { status: "logged_in" }
  | { status: "logged_out" }
  | { status: "config_invalid"; diagnostic: string }
  | { status: "not_applicable" }
  | { status: "unknown" };

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
  /** Provider facts owned by the runtime's Rust catalog. Empty when unsupported. */
  providerProfiles: AcpProviderProfile[];
  /** Environment variable used to apply thinking effort, when supported. */
  thinkingEnvVar: string | null;
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
  /** Compiled, PATH-probed, or user-defined catalog source. */
  source: "builtin" | "preset" | "custom";
  /** Definition env for custom entries; absent for builtin/preset entries. */
  definitionEnv?: Record<string, string>;
  /** Spawn-time parallelism cap; absent for uncapped harnesses. */
  maxParallelism?: number;
};

/** A catalog entry confirmed available with resolved command and binary. */
export type AcpRuntime = AcpRuntimeCatalogEntry & {
  availability: "available";
  command: string;
  binaryPath: string;
};
