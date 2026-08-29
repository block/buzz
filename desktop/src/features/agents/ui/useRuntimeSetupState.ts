import { useQuery } from "@tanstack/react-query";
import type { RuntimeSetupField } from "@/shared/api/types";
import { getRuntimeSetupSecretStatus } from "@/shared/api/runtimeSetupSecrets";
import type { EnvVarsValue } from "./EnvVarsEditor";
export { RuntimeSetupFields } from "./RuntimeSetupFields";

const runtimeSetupSecretStatusQueryKey = [
  "runtime-setup-secret-status",
] as const;

export function useRuntimeSetupState({
  agentPubkey,
  definitionId,
  disabled,
  fields = [],
  inheritedFrom,
  onChange,
  open,
  runtimeId,
  value,
}: {
  agentPubkey?: string | null;
  definitionId?: string | null;
  disabled?: boolean;
  fields?: readonly RuntimeSetupField[];
  inheritedFrom?: EnvVarsValue;
  onChange: (next: EnvVarsValue) => void;
  open: boolean;
  runtimeId: string;
  value: EnvVarsValue;
}) {
  const normalizedAgentPubkey = agentPubkey?.trim() || null;
  const normalizedDefinitionId = definitionId?.trim() || null;
  const query = useQuery({
    enabled:
      open &&
      runtimeId.trim().length > 0 &&
      fields.some((field) => field.kind === "secret") &&
      (normalizedAgentPubkey !== null || normalizedDefinitionId !== null),
    queryKey: [
      ...runtimeSetupSecretStatusQueryKey,
      runtimeId,
      normalizedDefinitionId,
      normalizedAgentPubkey,
    ],
    queryFn: () =>
      getRuntimeSetupSecretStatus({
        agentPubkey: normalizedAgentPubkey,
        definitionId: normalizedDefinitionId,
        runtimeId,
      }),
  });
  const configuredKeys = query.data?.configuredEnvKeys ?? [];
  const inheritedKeys = query.data?.inheritedEnvKeys ?? [];
  return {
    configuredKeys,
    envKeys: fields.map((field) => field.envKey),
    fieldProps: {
      configuredByKeys: configuredKeys,
      disabled,
      fields,
      inheritedFrom,
      inheritedConfiguredByKeys: inheritedKeys,
      inheritedLabel: normalizedAgentPubkey
        ? "linked agent definition"
        : undefined,
      secureStorageUnavailable: query.isError,
      onChange,
      value,
    },
    fields,
    inheritedKeys,
    queryFailed: query.isError,
    satisfiedKeys: [...new Set([...configuredKeys, ...inheritedKeys])],
  };
}

export function hiddenKeys(
  topLevelSecretEnvVar: string | null | undefined,
  setupEnvKeys: readonly string[],
): string[] {
  return topLevelSecretEnvVar
    ? [topLevelSecretEnvVar, ...setupEnvKeys]
    : [...setupEnvKeys];
}
