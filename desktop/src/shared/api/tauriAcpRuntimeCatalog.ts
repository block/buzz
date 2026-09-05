import type {
  AcpAvailabilityStatus,
  AcpRuntimeCatalogEntry,
  AuthStatus,
} from "@/shared/api/types";

export type RawAcpRuntimeCatalogEntry = {
  id: string;
  label: string;
  avatar_url: string;
  availability: AcpAvailabilityStatus;
  command: string | null;
  binary_path: string | null;
  default_args: string[];
  mcp_command: string | null;
  model_env_var?: string | null;
  provider_env_var?: string | null;
  thinking_env_var?: string | null;
  max_tokens_env_var?: string | null;
  context_limit_env_var?: string | null;
  max_rounds_env_var?: string | null;
  install_hint: string;
  install_instructions_url: string;
  can_auto_install: boolean;
  /** Optional only for older E2E fixtures; the Rust catalog always supplies it. */
  requires_external_cli?: boolean;
  underlying_cli_path: string | null;
  node_required: boolean;
  /** Tagged union with snake_case status values — same shape as `AuthStatus`. */
  auth_status: AuthStatus;
  login_hint?: string;
  source: "builtin" | "preset" | "custom";
  /**
   * Definition-level env vars for `source: custom` entries.
   * Omitted/absent for builtin and preset — skipped in Rust serialization when empty.
   */
  definition_env?: Record<string, string>;
  max_parallelism?: number;
  supports_acp_native_config?: boolean;
  supports_acp_model_switching?: boolean;
  mcp_hooks?: boolean;
};

export function fromRawAcpRuntimeCatalogEntry(
  entry: RawAcpRuntimeCatalogEntry,
): AcpRuntimeCatalogEntry {
  return {
    id: entry.id,
    label: entry.label,
    avatarUrl: entry.avatar_url,
    availability: entry.availability,
    command: entry.command,
    binaryPath: entry.binary_path,
    defaultArgs: entry.default_args,
    mcpCommand: entry.mcp_command,
    modelEnvVar: entry.model_env_var ?? null,
    providerEnvVar: entry.provider_env_var ?? null,
    thinkingEnvVar: entry.thinking_env_var ?? null,
    maxTokensEnvVar: entry.max_tokens_env_var ?? null,
    contextLimitEnvVar: entry.context_limit_env_var ?? null,
    maxRoundsEnvVar: entry.max_rounds_env_var ?? null,
    installHint: entry.install_hint,
    installInstructionsUrl: entry.install_instructions_url,
    canAutoInstall: entry.can_auto_install,
    requiresExternalCli: entry.requires_external_cli ?? false,
    underlyingCliPath: entry.underlying_cli_path,
    nodeRequired: entry.node_required,
    authStatus: entry.auth_status,
    loginHint: entry.login_hint ?? null,
    source: entry.source,
    // Absent when Rust skips an empty BTreeMap.
    definitionEnv: entry.definition_env ?? {},
    ...(entry.max_parallelism !== undefined && {
      maxParallelism: entry.max_parallelism,
    }),
    supportsAcpNativeConfig: entry.supports_acp_native_config ?? null,
    supportsAcpModelSwitching: entry.supports_acp_model_switching ?? null,
    mcpHooks: entry.mcp_hooks ?? null,
  };
}
