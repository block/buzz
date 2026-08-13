import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  enableCodexSharedRuntime,
  getCodexSharedRuntimeStatus,
  launchCodexDesktopShared,
  takeOverCodexDesktopShared,
} from "@/shared/api/codexTasks";
import { discoverAcpRuntimes, installAcpRuntime } from "@/shared/api/tauri";
import { getInstallErrorMessage } from "@/shared/lib/installError";

export const codexSharedRuntimeQueryKey = ["codex-shared-runtime"] as const;
const acpRuntimesQueryKey = ["acp-runtimes"] as const;
const managedAgentsQueryKey = ["managed-agents"] as const;

export function useCodexSharedRuntimeQuery(options?: { enabled?: boolean }) {
  return useQuery({
    enabled: options?.enabled ?? true,
    queryKey: codexSharedRuntimeQueryKey,
    queryFn: getCodexSharedRuntimeStatus,
    staleTime: 2_000,
    refetchInterval: 10_000,
  });
}

export function useEnableCodexSharedRuntimeMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: enableCodexSharedRuntime,
    onSuccess: (status) => {
      queryClient.setQueryData(codexSharedRuntimeQueryKey, status);
    },
  });
}

export function useSetupCodexSharedRuntimeMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const installResult = await installAcpRuntime("codex");
      if (!installResult.success) {
        throw new Error(getInstallErrorMessage(installResult));
      }
      queryClient.setQueryData(
        acpRuntimesQueryKey,
        await discoverAcpRuntimes(),
      );
      return enableCodexSharedRuntime();
    },
    onSuccess: (status) => {
      queryClient.setQueryData(codexSharedRuntimeQueryKey, status);
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: acpRuntimesQueryKey });
      void queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
    },
  });
}

export function useLaunchCodexDesktopSharedMutation() {
  return useMutation({ mutationFn: launchCodexDesktopShared });
}

export function useTakeOverCodexDesktopSharedMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: takeOverCodexDesktopShared,
    onSuccess: (status) => {
      queryClient.setQueryData(codexSharedRuntimeQueryKey, status);
    },
  });
}
