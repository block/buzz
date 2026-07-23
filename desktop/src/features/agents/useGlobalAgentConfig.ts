/**
 * React hook: load the global agent configuration defaults.
 *
 * Backed by TanStack Query with a stable query key so the config is fetched
 * once per QueryClient lifetime and shared across all callers — dialogs always
 * receive the already-populated value on first render, eliminating the
 * per-mount IPC race that caused required-env-key rows to be missing on open.
 *
 * On fetch error the query falls back to EMPTY_CONFIG (safe — the absence of
 * a global config is never an error state for callers).
 */
import * as React from "react";

import { useQuery } from "@tanstack/react-query";

import {
  maskDisabledAcpRuntimePreference,
  useDisabledAcpRuntimeIds,
} from "@/features/agents/lib/runtimeVisibilityPreference";
import { getGlobalAgentConfig } from "@/shared/api/tauriGlobalAgentConfig";
import type { GlobalAgentConfig } from "@/shared/api/types";

const EMPTY_CONFIG: GlobalAgentConfig = {
  env_vars: {},
  provider: null,
  model: null,
  preferred_runtime: null,
};

export const globalAgentConfigQueryKey = ["globalAgentConfig"] as const;

export function useGlobalAgentConfig(): {
  globalConfig: GlobalAgentConfig;
  isLoading: boolean;
} {
  const { data, isPending } = useQuery({
    queryKey: globalAgentConfigQueryKey,
    queryFn: getGlobalAgentConfig,
    // Config is only mutated via setGlobalAgentConfig — treat as stable until
    // explicitly invalidated by AgentDefaultsSettingsCard after a save.
    staleTime: Number.POSITIVE_INFINITY,
    // Never show a stale empty flash while a background refetch runs.
    placeholderData: EMPTY_CONFIG,
  });

  return {
    globalConfig: data ?? EMPTY_CONFIG,
    isLoading: isPending,
  };
}

/**
 * Load global defaults for a new implicit runtime choice.
 *
 * Existing agent edit surfaces must use useGlobalAgentConfig so a hidden
 * harness can still inherit its persisted provider and model. Start paths use
 * this hook to ignore a hidden preferred harness and its dependent defaults.
 */
export function useImplicitGlobalAgentConfig(): {
  globalConfig: GlobalAgentConfig;
  isLoading: boolean;
} {
  const { globalConfig, isLoading } = useGlobalAgentConfig();
  const disabledRuntimeIds = useDisabledAcpRuntimeIds();
  const implicitGlobalConfig = React.useMemo(
    () => maskDisabledAcpRuntimePreference(globalConfig, disabledRuntimeIds),
    [disabledRuntimeIds, globalConfig],
  );

  return {
    globalConfig: implicitGlobalConfig,
    isLoading,
  };
}
