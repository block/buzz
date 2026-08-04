import type { GlobalAgentConfig } from "@/shared/api/types";

export const GUARDIAN_POLICY_ENV = "BUZZ_ACP_PERMISSION_MODE";

const STRUCTURED_ENV_KEYS = new Set([
  "BUZZ_AGENT_PROVIDER",
  "BUZZ_AGENT_MODEL",
  "BUZZ_AGENT_THINKING_EFFORT",
  GUARDIAN_POLICY_ENV,
]);

export function getGenericEnvVars(envVars: Record<string, string>) {
  return Object.fromEntries(
    Object.entries(envVars).filter(([key]) => !STRUCTURED_ENV_KEYS.has(key)),
  );
}

export function mergeGenericEnvVars(
  current: Record<string, string>,
  nextGeneric: Record<string, string>,
) {
  const merged = { ...nextGeneric };
  for (const key of STRUCTURED_ENV_KEYS) {
    if (current[key] !== undefined) merged[key] = current[key];
  }
  return merged;
}

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
