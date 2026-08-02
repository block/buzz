import * as React from "react";
import { LogOut, ShieldCheck } from "lucide-react";

import {
  useClickUpConnection,
  useClickUpTasks,
  useClickUpWorkspaces,
  useConnectClickUp,
  useDisconnectClickUp,
} from "@/features/clickup/hooks";
import type { ClickUpTaskFilters } from "@/features/clickup/lib/tasks";
import { ClickUpApiError } from "@/features/clickup/types";
import { ClickUpConnectCard } from "@/features/clickup/ui/ClickUpConnectCard";
import { ClickUpTaskDetail } from "@/features/clickup/ui/ClickUpTaskDetail";
import { ClickUpTaskList } from "@/features/clickup/ui/ClickUpTaskList";
import { useIdentityQuery } from "@/shared/api/hooks";
import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/shared/ui/alert-dialog";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { PageHeader } from "@/shared/ui/PageHeader";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/shared/ui/sheet";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const EMPTY_FILTERS: ClickUpTaskFilters = {
  search: "",
  status: "all",
  priority: "all",
  location: "all",
  dueWindow: "all",
};

function workspaceStorageKey(pubkey: string) {
  return `buzz.clickup.last-workspace.v1:${pubkey.toLowerCase()}`;
}

function readStoredWorkspace(pubkey: string) {
  try {
    return window.localStorage.getItem(workspaceStorageKey(pubkey));
  } catch {
    return null;
  }
}

function writeStoredWorkspace(pubkey: string, workspaceId: string) {
  try {
    window.localStorage.setItem(workspaceStorageKey(pubkey), workspaceId);
  } catch {
    // Workspace preference is optional; task data and credentials are not stored here.
  }
}

function connectionErrorCopy(error: Error | null) {
  if (!(error instanceof ClickUpApiError)) return error?.message ?? null;
  if (error.code === "unauthorized" || error.code === "invalid_token")
    return "Buzz can no longer access this ClickUp account. Enter a valid personal token to reconnect.";
  if (error.code === "keyring_unavailable")
    return "Secure keyring storage is unavailable. Buzz will not store a ClickUp token without it.";
  if (error.code === "network")
    return "Buzz could not reach ClickUp. Check your connection and try again.";
  return error.message;
}

function retryAtFromError(error: unknown) {
  return error instanceof ClickUpApiError ? error.retryAtMs : null;
}

function requiresCredentialEntry(error: unknown) {
  return (
    error instanceof ClickUpApiError &&
    [
      "identity_changed",
      "invalid_token",
      "not_connected",
      "unauthorized",
    ].includes(error.code)
  );
}

function useNarrowLayout() {
  const [isNarrow, setIsNarrow] = React.useState(
    () => window.matchMedia("(max-width: 1023px)").matches,
  );

  React.useEffect(() => {
    const query = window.matchMedia("(max-width: 1023px)");
    const update = () => setIsNarrow(query.matches);
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);

  return isNarrow;
}

export function ClickUpScreen() {
  const identityQuery = useIdentityQuery();
  const pubkey = identityQuery.data?.pubkey;
  const connectionQuery = useClickUpConnection(pubkey);
  const connection = connectionQuery.data;
  const connectMutation = useConnectClickUp(pubkey);
  const disconnectMutation = useDisconnectClickUp(pubkey);
  const workspacesQuery = useClickUpWorkspaces(
    pubkey,
    connection?.connected ?? false,
  );
  const [selectedWorkspaceId, setSelectedWorkspaceId] = React.useState<
    string | undefined
  >();
  const [selectedTaskId, setSelectedTaskId] = React.useState<string | null>(
    null,
  );
  const [filters, setFilters] =
    React.useState<ClickUpTaskFilters>(EMPTY_FILTERS);
  const tasksQuery = useClickUpTasks(pubkey, selectedWorkspaceId);
  const retryAtMs = retryAtFromError(tasksQuery.error);
  const [retryClock, setRetryClock] = React.useState(() => Date.now());
  const isNarrow = useNarrowLayout();

  React.useEffect(() => {
    if (!retryAtMs || retryAtMs <= Date.now()) return;
    const timer = window.setTimeout(
      () => setRetryClock(Date.now()),
      retryAtMs - Date.now() + 50,
    );
    return () => window.clearTimeout(timer);
  }, [retryAtMs]);

  React.useEffect(() => {
    if (!pubkey || !workspacesQuery.data?.length) return;
    const available = workspacesQuery.data;
    const stored = readStoredWorkspace(pubkey);
    const next = available.some(
      (workspace) => workspace.id === selectedWorkspaceId,
    )
      ? selectedWorkspaceId
      : available.some((workspace) => workspace.id === stored)
        ? (stored ?? available[0]?.id)
        : available[0]?.id;
    if (next && next !== selectedWorkspaceId) setSelectedWorkspaceId(next);
  }, [pubkey, selectedWorkspaceId, workspacesQuery.data]);

  React.useEffect(() => {
    if (!pubkey || !selectedWorkspaceId) return;
    writeStoredWorkspace(pubkey, selectedWorkspaceId);
    setSelectedTaskId(null);
    setFilters(EMPTY_FILTERS);
  }, [pubkey, selectedWorkspaceId]);

  const connect = React.useCallback(
    async (token: string) => {
      await connectMutation.connect(token);
    },
    [connectMutation.connect],
  );

  if (identityQuery.isLoading || (connectionQuery.isLoading && !connection)) {
    return <ViewLoadingFallback kind="clickup" />;
  }

  const connectionError = connectionQuery.error ?? connectMutation.error;
  const connectionFailure = connectionErrorCopy(connectionError);

  if (
    !connection?.connected &&
    (!connectionQuery.error || requiresCredentialEntry(connectionError))
  ) {
    return (
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-tl-xl">
        <ClickUpConnectCard
          errorMessage={connectionFailure}
          isPending={connectMutation.isPending}
          onConnect={connect}
        />
      </div>
    );
  }

  if (!connection?.connected) {
    return (
      <div className="relative flex min-h-0 min-w-0 flex-1 items-center justify-center overflow-hidden rounded-tl-xl px-4 py-10">
        <Card className="w-full max-w-xl space-y-4 p-6" role="alert">
          <div className="space-y-1">
            <h1 className="text-xl font-semibold">
              ClickUp is temporarily unavailable
            </h1>
            <p className="text-sm text-muted-foreground">
              {connectionFailure ??
                "Buzz could not verify the local ClickUp connection."}
            </p>
          </div>
          <Button
            disabled={connectionQuery.isFetching}
            onClick={() => void connectionQuery.refetch()}
            variant="outline"
          >
            {connectionQuery.isFetching ? "Trying again…" : "Try again"}
          </Button>
        </Card>
      </div>
    );
  }

  const workspaces = workspacesQuery.data ?? [];
  const selectedWorkspace = workspaces.find(
    (workspace) => workspace.id === selectedWorkspaceId,
  );
  const tasksError =
    tasksQuery.error instanceof ClickUpApiError ? tasksQuery.error : null;

  return (
    <div
      className={cn(
        "relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-tl-xl",
        topChromeInset.divider,
      )}
    >
      <div className="buzz-content-scrollbar min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-scroll">
        <div className="px-4 pb-8 pt-7 sm:px-6 sm:pt-8">
          <div className="mx-auto w-full max-w-6xl space-y-6">
            <PageHeader
              action={
                <div className="flex flex-wrap items-center justify-end gap-2">
                  <Badge variant="success">
                    <ShieldCheck className="mr-1 h-3 w-3" />
                    Read-only
                  </Badge>
                  <AlertDialog>
                    <AlertDialogTrigger asChild>
                      <Button
                        disabled={disconnectMutation.isPending}
                        size="sm"
                        variant="ghost"
                      >
                        <LogOut />
                        Disconnect
                      </Button>
                    </AlertDialogTrigger>
                    <AlertDialogContent>
                      <AlertDialogHeader>
                        <AlertDialogTitle>Disconnect ClickUp?</AlertDialogTitle>
                        <AlertDialogDescription>
                          Buzz will remove the ClickUp token stored for this
                          Buzz identity. No ClickUp tasks or account settings
                          will be changed.
                        </AlertDialogDescription>
                      </AlertDialogHeader>
                      <AlertDialogFooter>
                        <AlertDialogCancel>Cancel</AlertDialogCancel>
                        <AlertDialogAction
                          onClick={() => disconnectMutation.mutate()}
                        >
                          Disconnect ClickUp
                        </AlertDialogAction>
                      </AlertDialogFooter>
                    </AlertDialogContent>
                  </AlertDialog>
                </div>
              }
              description="Your assigned ClickUp work, available as a reference surface inside Buzz."
              title="ClickUp"
            />

            {connectionQuery.error ? (
              <div
                className="flex flex-wrap items-center justify-between gap-3 rounded-xl bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300"
                role="alert"
              >
                <span>{connectionFailure}</span>
                <Button
                  disabled={connectionQuery.isFetching}
                  onClick={() => void connectionQuery.refetch()}
                  size="sm"
                  variant="outline"
                >
                  Try again
                </Button>
              </div>
            ) : null}

            {disconnectMutation.error ? (
              <div
                className="flex flex-wrap items-center justify-between gap-3 rounded-xl bg-destructive/10 px-3 py-2 text-xs text-destructive"
                role="alert"
              >
                <span>
                  ClickUp is still connected because Buzz could not remove the
                  local credential.
                </span>
                <Button
                  disabled={disconnectMutation.isPending}
                  onClick={() => disconnectMutation.mutate()}
                  size="sm"
                  variant="outline"
                >
                  Try disconnecting again
                </Button>
              </div>
            ) : null}

            <div className="flex flex-col gap-3 rounded-xl border border-border/60 bg-card/40 p-3 sm:flex-row sm:items-center sm:justify-between">
              <div className="min-w-0">
                <p className="truncate text-sm font-medium">
                  {connection.account?.username ?? "Connected account"}
                </p>
                <p className="truncate text-xs text-muted-foreground">
                  {connection.account?.email ?? "ClickUp personal token"}
                </p>
              </div>
              <label className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
                <span className="shrink-0">Workspace</span>
                <select
                  aria-label="Select ClickUp Workspace"
                  className="h-8 min-w-0 max-w-xs rounded-lg border border-input/40 bg-background px-2 text-xs text-foreground outline-hidden focus:ring-1 focus:ring-ring"
                  disabled={
                    workspacesQuery.isLoading || workspaces.length === 0
                  }
                  onChange={(event) =>
                    setSelectedWorkspaceId(event.target.value)
                  }
                  value={selectedWorkspaceId ?? ""}
                >
                  {workspaces.map((workspace) => (
                    <option key={workspace.id} value={workspace.id}>
                      {workspace.name}
                    </option>
                  ))}
                </select>
              </label>
            </div>

            {workspacesQuery.isError ? (
              <div
                className="flex flex-wrap items-center justify-between gap-3 rounded-xl bg-destructive/10 px-3 py-2 text-xs text-destructive"
                role="alert"
              >
                <span>Could not load authorized ClickUp Workspaces.</span>
                <Button
                  disabled={workspacesQuery.isFetching}
                  onClick={() => void workspacesQuery.refetch()}
                  size="sm"
                  variant="outline"
                >
                  Try again
                </Button>
              </div>
            ) : null}

            {workspacesQuery.isSuccess && workspaces.length === 0 ? (
              <div className="rounded-xl bg-muted/50 px-4 py-8 text-center text-sm">
                This connected account has no available Workspaces.
              </div>
            ) : null}

            {selectedWorkspace ? (
              <div className="grid min-w-0 gap-4 lg:grid-cols-3">
                <ClickUpTaskList
                  error={tasksError}
                  fetchedAtMs={tasksQuery.data?.fetchedAtMs ?? null}
                  filters={filters}
                  isFetching={tasksQuery.isFetching}
                  isLoading={tasksQuery.isLoading}
                  onFiltersChange={setFilters}
                  onRefresh={() => void tasksQuery.refetch()}
                  onSelectTask={setSelectedTaskId}
                  retryReady={!retryAtMs || retryClock >= retryAtMs}
                  selectedTaskId={selectedTaskId}
                  tasks={tasksQuery.data?.tasks ?? []}
                  truncated={tasksQuery.data?.truncated ?? false}
                />
                {selectedTaskId && pubkey && !isNarrow ? (
                  <ClickUpTaskDetail
                    onClose={() => setSelectedTaskId(null)}
                    pubkey={pubkey}
                    taskId={selectedTaskId}
                    workspaceId={selectedWorkspace.id}
                  />
                ) : (
                  <aside className="hidden lg:block">
                    <div className="sticky top-4 rounded-xl border border-dashed border-border/70 p-6 text-center text-sm text-muted-foreground">
                      Select a task to inspect its read-only details.
                    </div>
                  </aside>
                )}
                {pubkey && isNarrow ? (
                  <Sheet
                    onOpenChange={(open) => {
                      if (!open) setSelectedTaskId(null);
                    }}
                    open={Boolean(selectedTaskId)}
                  >
                    <SheetContent
                      className="w-full overflow-y-auto p-4 sm:max-w-xl"
                      side="right"
                    >
                      <SheetHeader className="sr-only">
                        <SheetTitle>ClickUp task details</SheetTitle>
                        <SheetDescription>
                          Read-only details for the selected ClickUp task.
                        </SheetDescription>
                      </SheetHeader>
                      {selectedTaskId ? (
                        <ClickUpTaskDetail
                          onClose={() => setSelectedTaskId(null)}
                          pubkey={pubkey}
                          taskId={selectedTaskId}
                          workspaceId={selectedWorkspace.id}
                        />
                      ) : null}
                    </SheetContent>
                  </Sheet>
                ) : null}
              </div>
            ) : null}
          </div>
        </div>
      </div>
    </div>
  );
}
