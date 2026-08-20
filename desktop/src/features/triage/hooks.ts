import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  createTodo,
  fetchSuggestions,
  fetchTodos,
  scanCandidates,
  sendFeedback,
  updateTodo,
  type TriageFeedback,
  type TriageSuggestion,
  type TriageTodo,
  type TriageTodoStatus,
} from "@/features/triage/api";
import type { TriageCandidate } from "@/features/triage/lib/collectCandidates";

/** Keyed by pubkey so a different identity never reads another's triage state. */
export const triageQueryKeys = {
  suggestions: (pubkey: string | undefined) =>
    ["triage", "suggestions", pubkey ?? "anonymous"] as const,
  todos: (pubkey: string | undefined) =>
    ["triage", "todos", pubkey ?? "anonymous"] as const,
};

export function useTriageSuggestionsQuery(pubkey: string | undefined) {
  return useQuery({
    enabled: Boolean(pubkey),
    queryKey: triageQueryKeys.suggestions(pubkey),
    queryFn: async () => (await fetchSuggestions(pubkey as string)).suggestions,
    staleTime: 60_000,
  });
}

export function useTriageTodosQuery(pubkey: string | undefined) {
  return useQuery({
    enabled: Boolean(pubkey),
    queryKey: triageQueryKeys.todos(pubkey),
    queryFn: async () => (await fetchTodos(pubkey as string)).todos,
    staleTime: 30_000,
  });
}

export function useTriageScanMutation(pubkey: string | undefined) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (candidates: TriageCandidate[]) => {
      if (!pubkey) throw new Error("No identity available for triage");
      return (await scanCandidates({ pubkey, candidates })).suggestions;
    },
    onSuccess: (suggestions) => {
      queryClient.setQueryData<TriageSuggestion[]>(
        triageQueryKeys.suggestions(pubkey),
        suggestions,
      );
    },
  });
}

export function useAdoptTodoMutation(pubkey: string | undefined) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (input: {
      eventId: string;
      channelId: string | null;
      channelName: string | null;
      threadRootId: string | null;
      authorLabel: string | null;
      preview: string;
      reason: string;
    }) => {
      if (!pubkey) throw new Error("No identity available for triage");
      return (await createTodo({ ...input, pubkey })).todo;
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: triageQueryKeys.todos(pubkey),
      });
    },
  });
}

export function useResolveTodoMutation(pubkey: string | undefined) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (input: { id: string; status: TriageTodoStatus }) => {
      if (!pubkey) throw new Error("No identity available for triage");
      return (await updateTodo({ ...input, pubkey })).todo;
    },
    onMutate: async ({ id, status }) => {
      const key = triageQueryKeys.todos(pubkey);
      const previous = queryClient.getQueryData<TriageTodo[]>(key);
      queryClient.setQueryData<TriageTodo[]>(key, (todos) =>
        (todos ?? []).map((todo) =>
          todo.id === id ? { ...todo, status } : todo,
        ),
      );
      return { key, previous };
    },
    onError: (_error, _input, context) => {
      if (context?.previous) {
        queryClient.setQueryData(context.key, context.previous);
      }
    },
    onSettled: () => {
      void queryClient.invalidateQueries({
        queryKey: triageQueryKeys.todos(pubkey),
      });
    },
  });
}

/**
 * Feedback is also what persists a decision: the service rewrites the stored
 * verdict, so the suggestions query must be refetched for the change to show.
 */
export function useTriageFeedbackMutation(pubkey: string | undefined) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: TriageFeedback) => sendFeedback(input),
    onSettled: () => {
      void queryClient.invalidateQueries({
        queryKey: triageQueryKeys.suggestions(pubkey),
      });
    },
  });
}
