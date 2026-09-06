export type AgentStatus = "running" | "stopped" | "deployed" | "not_deployed";

export type ManagedAgent = {
  pubkey: string;
  name: string;
  runtime: string | null;
  agentCommand: string;
  systemPrompt: string | null;
  model: string | null;
  provider: string | null;
  status: AgentStatus;
  lastError: string | null;
};

export type RuntimeAvailability =
  | "available"
  | "adapter_missing"
  | "adapter_outdated"
  | "cli_missing"
  | "not_installed";

export type RuntimeAuthStatus =
  | { status: "logged_in" }
  | { status: "logged_out" }
  | { status: "config_invalid"; diagnostic: string }
  | { status: "not_applicable" }
  | { status: "unknown" };

export type AgentRuntime = {
  id: string;
  label: string;
  availability: RuntimeAvailability;
  command: string | null;
  authStatus: RuntimeAuthStatus;
  installHint: string;
  loginHint: string | null;
};
