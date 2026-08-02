import type { GlobalAgentConfig } from "@/shared/api/types";

export const GUARDIAN_POLICY_ENV = "BUZZ_ACP_PERMISSION_MODE";

export const GUARDIAN_POLICY_OPTIONS = [
  { label: "Monitor", value: "default" },
  { label: "Lockdown", value: "dont-ask" },
] as const;

export function applyGuardianPolicy(
  config: GlobalAgentConfig,
  value: string,
): GlobalAgentConfig {
  return {
    ...config,
    env_vars: { ...config.env_vars, [GUARDIAN_POLICY_ENV]: value },
  };
}
