import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  annotateFeedback,
  emptyFibresResponse,
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
import {
  moveFibreStatus,
  normalizeFibresResponse,
  type FibresPayload,
} from "@/features/triage/fibreStatus";

/** Keyed by pubkey so a different identity never reads another's fibres. */
export const fibreQueryKeys = {
  fibres: (pubkey: string | undefined) =>
    ["triage", "fibres", pubkey ?? "anonymous"] as const,
};

export function useFibresQuery(pubkey: string | undefined) {
  const queryClient = useQueryClient();
  return useQuery({
    enabled: Boolean(pubkey),
    queryKey: fibreQueryKeys.fibres(pubkey),
    queryFn: async () => {
      const payload = await fetchFibres(pubkey as string);
      return normalizeFibresResponse(
        payload,
        queryClient.getQueryData<FibresResponse>(fibreQueryKeys.fibres(pubkey)),
      );
    },
    staleTime: 5_000,
    refetchInterval: 15_000,
    retry: 1,
  });
}

function applyFibresPayload(
  queryClient: ReturnType<typeof useQueryClient>,
  pubkey: string | undefined,
  payload: FibresPayload,
) {
  queryClient.setQueryData<FibresResponse>(
    fibreQueryKeys.fibres(pubkey),
    (current) => normalizeFibresResponse(payload, current),
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
          ) ?? emptyFibresResponse()
        );
      }
      return ingestMessages({ pubkey, messages });
    },
    onSuccess: (payload) => {
      applyFibresPayload(queryClient, pubkey, payload);
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
        return moveFibreStatus(current, id, status);
      });
      return { key, previous };
    },
    onError: (_error, _input, context) => {
      if (context?.previous) {
        queryClient.setQueryData(context.key, context.previous);
      }
    },
    onSuccess: (payload) => {
      applyFibresPayload(queryClient, pubkey, payload);
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
    onSuccess: (payload) => {
      applyFibresPayload(queryClient, pubkey, payload);
    },
  });
}

export function useFibreFeedbackMutation() {
  return useMutation({
    mutationFn: (input: FibreFeedback) => sendFeedback(input),
  });
}

/** Attaches a written reason to feedback already recorded. */
export function useAnnotateFeedbackMutation() {
  return useMutation({
    mutationFn: (input: { pubkey: string; feedbackId: string; note: string }) =>
      annotateFeedback(input),
  });
}

export function selectOpenFibres(
  response: FibresResponse | undefined,
): Fibre[] {
  return response?.fibres ?? [];
}
