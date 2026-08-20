import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  fetchFibres,
  ingestMessages,
  patchFibre,
  restoreFibres,
  sendFeedback,
  type Fibre,
  type FibreFeedback,
  type FibreIngestMessage,
  type FibreStatus,
  type FibresResponse,
} from "@/features/triage/api";

/** Keyed by pubkey so a different identity never reads another's fibres. */
export const fibreQueryKeys = {
  fibres: (pubkey: string | undefined) =>
    ["triage", "fibres", pubkey ?? "anonymous"] as const,
};

export function useFibresQuery(pubkey: string | undefined) {
  return useQuery({
    enabled: Boolean(pubkey),
    queryKey: fibreQueryKeys.fibres(pubkey),
    queryFn: async () => fetchFibres(pubkey as string),
    staleTime: 5_000,
    refetchInterval: 15_000,
    retry: 1,
  });
}

function applyFibresResponse(
  queryClient: ReturnType<typeof useQueryClient>,
  pubkey: string | undefined,
  response: FibresResponse,
) {
  queryClient.setQueryData<FibresResponse>(
    fibreQueryKeys.fibres(pubkey),
    response,
  );
}

export function useIngestMessagesMutation(pubkey: string | undefined) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (messages: FibreIngestMessage[]) => {
      if (!pubkey) throw new Error("No identity available for fibre ingest");
      if (messages.length === 0) {
        return (
          queryClient.getQueryData<FibresResponse>(
            fibreQueryKeys.fibres(pubkey),
          ) ?? { fibres: [], openCount: 0, clearedCount: 0, ingested: 0 }
        );
      }
      return ingestMessages({ pubkey, messages });
    },
    onSuccess: (response) => {
      applyFibresResponse(queryClient, pubkey, response);
    },
  });
}

export function usePatchFibreMutation(pubkey: string | undefined) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (input: { id: string; status: FibreStatus }) => {
      if (!pubkey) throw new Error("No identity available for fibre ingest");
      return patchFibre({ ...input, pubkey });
    },
    onMutate: async ({ id, status }) => {
      const key = fibreQueryKeys.fibres(pubkey);
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<FibresResponse>(key);
      queryClient.setQueryData<FibresResponse>(key, (current) => {
        if (!current) return current;
        const fibres = current.fibres.filter((fibre) =>
          fibre.id === id ? status === "open" : true,
        );
        const wasOpen = current.fibres.some(
          (fibre) => fibre.id === id && fibre.status === "open",
        );
        const clearedDelta = status === "open" ? -1 : wasOpen ? 1 : 0;
        return {
          ...current,
          fibres,
          openCount: fibres.length,
          clearedCount: Math.max(0, current.clearedCount + clearedDelta),
        };
      });
      return { key, previous };
    },
    onError: (_error, _input, context) => {
      if (context?.previous) {
        queryClient.setQueryData(context.key, context.previous);
      }
    },
    onSuccess: (response) => {
      applyFibresResponse(queryClient, pubkey, response);
    },
  });
}

export function useRestoreFibresMutation(pubkey: string | undefined) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async () => {
      if (!pubkey) throw new Error("No identity available for fibre ingest");
      return restoreFibres(pubkey);
    },
    onSuccess: (response) => {
      applyFibresResponse(queryClient, pubkey, response);
    },
  });
}

export function useFibreFeedbackMutation() {
  return useMutation({
    mutationFn: (input: FibreFeedback) => sendFeedback(input),
  });
}

export function selectOpenFibres(
  response: FibresResponse | undefined,
): Fibre[] {
  return response?.fibres ?? [];
}
