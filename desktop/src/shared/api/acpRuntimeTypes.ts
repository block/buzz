export type AcpAvailabilityStatus =
  | "available"
  | "adapter_missing"
  | "adapter_outdated"
  | "cli_outdated"
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
  /** Detected vendor CLI version, when this runtime has a version gate. */
  cliVersion: string | null;
  /** Minimum vendor CLI version required by this Buzz build, when gated. */
  minimumCliVersion: string | null;
  defaultArgs: string[];
  mcpCommand: string | null;
  /** Environment variable used to apply the initial model, when supported. */
  modelEnvVar: string | null;
  /** Environment variable used to apply the selected LLM provider, when supported. */
  providerEnvVar: string | null;
  /** Environment variable used to apply thinking effort, when supported. */
  thinkingEnvVar: string | null;
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
  /**
   * Whether this entry is compiled into the app ("builtin"), a bundled preset
   * ("preset" — PATH-probed, not editable/deletable), or loaded from a user
   * JSON file in `custom_harnesses/` ("custom"). Controls editability in the
   * UI — only "custom" entries can be edited or deleted.
   */
  source: "builtin" | "preset" | "custom";
  /**
   * Definition-level environment variables for `source: custom` entries.
   *
   * Populated by the backend from `HarnessDefinition.env` so the edit form can
   * read them back without losing existing env vars on save. Always absent/empty
   * for `builtin` and `preset` entries.
   */
  definitionEnv?: Record<string, string>;
};

/** An AcpRuntimeCatalogEntry that is confirmed available — command and binaryPath are non-null. */
export type AcpRuntime = AcpRuntimeCatalogEntry & {
  availability: "available";
  command: string;
  binaryPath: string;
};
