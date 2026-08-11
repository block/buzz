import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  enableCodexSharedRuntime,
  getCodexSharedRuntimeStatus,
  launchCodexDesktopShared,
} from "@/shared/api/codexTasks";

export const codexSharedRuntimeQueryKey = ["codex-shared-runtime"] as const;

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

export function useLaunchCodexDesktopSharedMutation() {
  return useMutation({ mutationFn: launchCodexDesktopShared });
}
