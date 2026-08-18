import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  checkNxtlinqAuthorizationSetup,
  discoverNxtlinqAuthorizationGateway,
  getNxtlinqAuthorizationConfig,
  installNxtlinqAuthorizationGateway,
  uninstallNxtlinqAuthorizationGateway,
  setNxtlinqAuthorizationConfig,
} from "@/shared/api/tauriNxtlinq";

export const nxtlinqAuthorizationGatewayQueryKey = [
  "nxtlinq-authorization-gateway",
] as const;

export const nxtlinqAuthorizationConfigQueryKey = [
  "nxtlinq-authorization-config",
] as const;

export function useNxtlinqAuthorizationConfigQuery() {
  return useQuery({
    queryKey: nxtlinqAuthorizationConfigQueryKey,
    queryFn: getNxtlinqAuthorizationConfig,
    staleTime: 30_000,
  });
}

export function useSaveNxtlinqAuthorizationConfigMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: setNxtlinqAuthorizationConfig,
    onSuccess: (config) => {
      queryClient.setQueryData(nxtlinqAuthorizationConfigQueryKey, config);
    },
  });
}

export function useNxtlinqAuthorizationGatewayQuery(options?: {
  enabled?: boolean;
}) {
  return useQuery({
    enabled: options?.enabled ?? true,
    queryKey: nxtlinqAuthorizationGatewayQueryKey,
    queryFn: discoverNxtlinqAuthorizationGateway,
    staleTime: 30_000,
  });
}

export function useInstallNxtlinqAuthorizationGatewayMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (force: boolean) => installNxtlinqAuthorizationGateway(force),
    onSettled: () => {
      void queryClient.invalidateQueries({
        queryKey: nxtlinqAuthorizationGatewayQueryKey,
      });
    },
  });
}

export function useUninstallNxtlinqAuthorizationGatewayMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: uninstallNxtlinqAuthorizationGateway,
    onSettled: () => {
      void queryClient.invalidateQueries({
        queryKey: nxtlinqAuthorizationGatewayQueryKey,
      });
    },
  });
}

export function useNxtlinqAuthorizationSetupQuery(input: {
  projectRoot: string;
  trustStore: string;
  receiptDirectory: string;
  enabled?: boolean;
}) {
  return useQuery({
    enabled:
      (input.enabled ?? true) &&
      input.projectRoot.trim().length > 0 &&
      input.trustStore.trim().length > 0 &&
      input.receiptDirectory.trim().length > 0,
    queryKey: [
      ...nxtlinqAuthorizationGatewayQueryKey,
      "setup",
      input.projectRoot,
      input.trustStore,
      input.receiptDirectory,
    ],
    queryFn: () => checkNxtlinqAuthorizationSetup(input),
    staleTime: 5_000,
  });
}
