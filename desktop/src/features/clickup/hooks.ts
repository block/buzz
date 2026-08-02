import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { ClickUpApiError } from "@/features/clickup/types";
import {
  connectClickUp,
  disconnectClickUp,
  getClickUpConnection,
  getClickUpTask,
  getClickUpTaskComments,
  listClickUpTasks,
  listClickUpWorkspaces,
} from "@/shared/api/tauriClickUp";

export const clickUpKeys = {
  root: (pubkey: string) => ["clickup", pubkey] as const,
  connection: (pubkey: string) =>
    [...clickUpKeys.root(pubkey), "connection"] as const,
  workspaces: (pubkey: string) =>
    [...clickUpKeys.root(pubkey), "workspaces"] as const,
  tasks: (pubkey: string, workspaceId: string) =>
    [...clickUpKeys.root(pubkey), "workspace", workspaceId, "tasks"] as const,
  task: (pubkey: string, workspaceId: string, taskId: string) =>
    [
      ...clickUpKeys.root(pubkey),
      "workspace",
      workspaceId,
      "task",
      taskId,
    ] as const,
  comments: (pubkey: string, workspaceId: string, taskId: string) =>
    [
      ...clickUpKeys.root(pubkey),
      "workspace",
      workspaceId,
      "task",
      taskId,
      "comments",
    ] as const,
};

function retryClickUpQuery(failureCount: number, error: Error) {
  if (
    error instanceof ClickUpApiError &&
    [
      "forbidden",
      "invalid_token",
      "keyring_unavailable",
      "not_connected",
      "rate_limited",
      "unauthorized",
    ].includes(error.code)
  ) {
    return false;
  }
  return failureCount < 1;
}

export function useClickUpConnection(pubkey: string | undefined) {
  return useQuery({
    queryKey: clickUpKeys.connection(pubkey ?? "pending"),
    queryFn: getClickUpConnection,
    enabled: Boolean(pubkey),
    staleTime: 60_000,
    retry: retryClickUpQuery,
  });
}

export function useConnectClickUp(pubkey: string | undefined) {
  const queryClient = useQueryClient();
  const [isPending, setIsPending] = React.useState(false);
  const [error, setError] = React.useState<Error | null>(null);

  const connect = React.useCallback(
    async (personalToken: string) => {
      setIsPending(true);
      setError(null);
      try {
        const connection = await connectClickUp(personalToken);
        if (pubkey) {
          queryClient.setQueryData(clickUpKeys.connection(pubkey), connection);
          void queryClient.invalidateQueries({
            queryKey: clickUpKeys.workspaces(pubkey),
          });
        }
      } catch (caught) {
        const nextError =
          caught instanceof Error ? caught : new Error(String(caught));
        setError(nextError);
        throw nextError;
      } finally {
        setIsPending(false);
      }
    },
    [pubkey, queryClient],
  );

  return { connect, error, isPending };
}

export function useDisconnectClickUp(pubkey: string | undefined) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: disconnectClickUp,
    onSuccess: () => {
      if (!pubkey) return;
      queryClient.removeQueries({ queryKey: clickUpKeys.root(pubkey) });
      queryClient.setQueryData(clickUpKeys.connection(pubkey), {
        connected: false,
        account: null,
      });
    },
  });
}

export function useClickUpWorkspaces(
  pubkey: string | undefined,
  connected: boolean,
) {
  return useQuery({
    queryKey: clickUpKeys.workspaces(pubkey ?? "pending"),
    queryFn: listClickUpWorkspaces,
    enabled: Boolean(pubkey && connected),
    staleTime: 5 * 60_000,
    retry: retryClickUpQuery,
  });
}

export function useClickUpTasks(
  pubkey: string | undefined,
  workspaceId: string | undefined,
) {
  return useQuery({
    queryKey: clickUpKeys.tasks(pubkey ?? "pending", workspaceId ?? "pending"),
    queryFn: () => listClickUpTasks(workspaceId ?? ""),
    enabled: Boolean(pubkey && workspaceId),
    staleTime: 60_000,
    retry: retryClickUpQuery,
  });
}

export function useClickUpTask(
  pubkey: string | undefined,
  workspaceId: string | undefined,
  taskId: string | null,
) {
  return useQuery({
    queryKey: clickUpKeys.task(
      pubkey ?? "pending",
      workspaceId ?? "pending",
      taskId ?? "pending",
    ),
    queryFn: () => getClickUpTask(workspaceId ?? "", taskId ?? ""),
    enabled: Boolean(pubkey && workspaceId && taskId),
    staleTime: 60_000,
    retry: retryClickUpQuery,
  });
}

export function useClickUpTaskComments(
  pubkey: string | undefined,
  workspaceId: string | undefined,
  taskId: string | null,
) {
  return useQuery({
    queryKey: clickUpKeys.comments(
      pubkey ?? "pending",
      workspaceId ?? "pending",
      taskId ?? "pending",
    ),
    queryFn: () => getClickUpTaskComments(workspaceId ?? "", taskId ?? ""),
    enabled: Boolean(pubkey && workspaceId && taskId),
    staleTime: 60_000,
    retry: retryClickUpQuery,
  });
}
