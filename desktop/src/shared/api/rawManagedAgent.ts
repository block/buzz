import type { ManagedAgent, ManagedAgentBackend } from "./managedAgentTypes";
import type { RestartDiffEntry as RawRestartDiffEntry } from "./restartDiff";

export type RawManagedAgent = {
  pubkey: string;
  name: string;
  persona_id: string | null;
  // Optional: pre-feature fixtures may omit it. The record's harness/runtime id.
  runtime?: string | null;
  team_id?: string | null;
  relay_url: string;
  acp_command: string;
  agent_command: string;
  agent_command_override?: string | null;
  agent_args: string[];
  mcp_command: string;
  turn_timeout_seconds: number;
  idle_timeout_seconds: number | null;
  max_turn_duration_seconds: number | null;
  parallelism: number;
  system_prompt: string | null;
  avatar_url?: string | null;
  model: string | null;
  model_source?: ManagedAgent["modelSource"];
  provider: string | null;
  persona_out_of_date: boolean;
  persona_orphaned: boolean;
  needs_restart: boolean;
  restart_diff?: RawRestartDiffEntry[];
  env_vars?: Record<string, string>;
  status: ManagedAgent["status"];
  pid: number | null;
  created_at: string;
  updated_at: string;
  last_started_at: string | null;
  last_stopped_at: string | null;
  last_exit_code: number | null;
  last_error: string | null;
  last_error_code: number | null;
  log_path: string;
  start_on_app_launch: boolean;
  auto_restart_on_config_change?: boolean;
  backend: ManagedAgentBackend;
  backend_agent_id: string | null;
  // Pre-feature fixtures may omit these; mapped to "owner-only"/[] in fromRawManagedAgent.
  respond_to?: ManagedAgent["respondTo"];
  respond_to_allowlist?: string[];
  heartbeat_preflight?: {
    policy_file: string;
    policy_sha256: string;
    heartbeat_interval_seconds: number;
  } | null;
};
