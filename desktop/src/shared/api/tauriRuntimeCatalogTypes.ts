import type { AcpAvailabilityStatus, AuthStatus } from "./runtimeCatalogTypes";
import type { RawAcpProviderProfile } from "./providerProfiles";

/** Snake-case runtime catalog shape returned by Tauri. */
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
  provider_profiles?: RawAcpProviderProfile[];
  thinking_env_var?: string | null;
  max_tokens_env_var?: string | null;
  context_limit_env_var?: string | null;
  max_rounds_env_var?: string | null;
  install_hint: string;
  install_instructions_url: string;
  can_auto_install: boolean;
  /** Optional only for older E2E fixtures; Rust always supplies it. */
  requires_external_cli?: boolean;
  underlying_cli_path: string | null;
  node_required: boolean;
  auth_status: AuthStatus;
  login_hint?: string;
  source: "builtin" | "preset" | "custom";
  definition_env?: Record<string, string>;
  max_parallelism?: number;
};
