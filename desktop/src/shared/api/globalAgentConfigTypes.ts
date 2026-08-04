/**
 * Global agent defaults. Precedence:
 * baked floor < global < persona < per-agent.
 */
export type GlobalAgentConfig = {
  env_vars: Record<string, string>;
  provider: string | null;
  model: string | null;
  preferred_runtime: string | null;
};

export type GlobalAgentConfigSaveResult = {
  config: GlobalAgentConfig;
  restarted_count: number;
  failed_restart_count: number;
};
