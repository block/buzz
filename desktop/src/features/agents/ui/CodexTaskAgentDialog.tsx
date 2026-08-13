import * as React from "react";
import { toast } from "sonner";

import {
  useAttachManagedAgentToChannelMutation,
  useAvailableAcpRuntimes,
  useCodexTasksQuery,
  useCreateManagedAgentMutation,
} from "@/features/agents/hooks";
import { useCodexSharedRuntimeQuery } from "@/features/agents/codexSharedRuntimeHooks";
import { isCodexSharedRuntimeUsable } from "@/features/agents/codexSharedRuntimeStatus";
import { useChannelsQuery } from "@/features/channels/hooks";
import type { CodexTaskSummary } from "@/shared/api/codexTaskTypes";
import type { ManagedAgent } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { CodexSharedRuntimePanel } from "./CodexSharedRuntimePanel";

function taskLabel(task: CodexTaskSummary) {
  return task.threadName.trim() || `Codex task ${task.id.slice(0, 8)}`;
}

export function CodexTaskAgentDialog({
  onCreated,
  onOpenChange,
  open,
}: {
  onCreated: (agent: ManagedAgent) => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const sharedRuntimeQuery = useCodexSharedRuntimeQuery({ enabled: open });
  const sharedRuntimeReady = isCodexSharedRuntimeUsable(
    sharedRuntimeQuery.data,
  );
  const runtimesQuery = useAvailableAcpRuntimes({ enabled: open });
  const codexRuntime = (runtimesQuery.data ?? []).find(
    (runtime) => runtime.id === "codex",
  );
  const codexSetupReady = sharedRuntimeReady && Boolean(codexRuntime);
  const tasksQuery = useCodexTasksQuery({
    enabled: open && codexSetupReady,
  });
  const channelsQuery = useChannelsQuery({ enabled: open });
  const createMutation = useCreateManagedAgentMutation();
  const attachMutation = useAttachManagedAgentToChannelMutation(null);
  const [search, setSearch] = React.useState("");
  const [taskId, setTaskId] = React.useState("");
  const [name, setName] = React.useState("");
  const [channelId, setChannelId] = React.useState("");

  const tasks = tasksQuery.data ?? [];
  const filteredTasks = React.useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    if (!query) return tasks;
    return tasks.filter((task) =>
      [task.threadName, task.workspace, task.id].some((value) =>
        value.toLocaleLowerCase().includes(query),
      ),
    );
  }, [search, tasks]);
  const selectedTask = tasks.find((task) => task.id === taskId) ?? null;
  const channels = React.useMemo(
    () =>
      (channelsQuery.data ?? []).filter(
        (channel) => channel.channelType !== "dm" && !channel.archivedAt,
      ),
    [channelsQuery.data],
  );
  const selectedChannel =
    channels.find((channel) => channel.id === channelId) ?? null;
  React.useEffect(() => {
    if (!open || taskId || tasks.length === 0) return;
    setTaskId(tasks[0].id);
    setName(taskLabel(tasks[0]));
  }, [open, taskId, tasks]);

  React.useEffect(() => {
    if (!open || filteredTasks.length === 0) return;

    const query = search.trim().toLocaleLowerCase();
    const exactMatch = query
      ? tasks.find((task) => task.id.toLocaleLowerCase() === query)
      : null;
    const selectedIsVisible = filteredTasks.some((task) => task.id === taskId);
    const nextTask =
      exactMatch ?? (!selectedIsVisible ? filteredTasks[0] : null);

    if (nextTask && nextTask.id !== taskId) {
      setTaskId(nextTask.id);
      setName(taskLabel(nextTask));
    }
  }, [filteredTasks, open, search, taskId, tasks]);

  function reset() {
    setSearch("");
    setTaskId("");
    setName("");
    setChannelId("");
    createMutation.reset();
    attachMutation.reset();
  }

  function handleOpenChange(next: boolean) {
    if (!next) reset();
    onOpenChange(next);
  }

  function selectTask(nextTaskId: string) {
    setTaskId(nextTaskId);
    const task = tasks.find((candidate) => candidate.id === nextTaskId);
    if (task) setName(taskLabel(task));
  }

  async function handleSubmit() {
    if (!selectedTask || !codexRuntime || !name.trim()) return;
    try {
      const created = await createMutation.mutateAsync({
        name: name.trim(),
        codexTaskId: selectedTask.id,
        codexAppServerUrl: sharedRuntimeQuery.data?.url,
        agentCommand: codexRuntime.command,
        agentArgs: codexRuntime.defaultArgs,
        avatarUrl: codexRuntime.avatarUrl,
        parallelism: 1,
        spawnAfterCreate: false,
        startOnAppLaunch: false,
        backend: { type: "local" },
        respondTo: "owner-only",
      });
      let agent = created.agent;
      let channelAttached = false;
      if (selectedChannel) {
        try {
          const attached = await attachMutation.mutateAsync({
            agent,
            channelId: selectedChannel.id,
            ensureRunning: false,
            role: "bot",
          });
          agent = attached.agent;
          channelAttached = true;
        } catch (cause) {
          toast.warning("Agent created without channel membership", {
            description:
              cause instanceof Error ? cause.message : "Could not add agent.",
          });
        }
      }
      toast.success(
        selectedChannel && channelAttached
          ? `Codex task added to #${selectedChannel.name}`
          : "Codex task added as an offline agent",
      );
      if (created.profileSyncError) toast.warning(created.profileSyncError);
      onCreated(agent);
      handleOpenChange(false);
    } catch {
      // The mutation owns the rendered error state.
    }
  }

  const error =
    sharedRuntimeQuery.error instanceof Error
      ? sharedRuntimeQuery.error
      : tasksQuery.error instanceof Error
        ? tasksQuery.error
        : runtimesQuery.error instanceof Error
          ? runtimesQuery.error
          : channelsQuery.error instanceof Error
            ? channelsQuery.error
            : createMutation.error instanceof Error
              ? createMutation.error
              : attachMutation.error instanceof Error
                ? attachMutation.error
                : null;

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <DialogContent className="max-w-2xl overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="shrink-0 border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>Add a Codex task as an agent</DialogTitle>
            <DialogDescription>
              Create an independent Buzz identity that resumes one existing
              Codex task across every Channel it joins.
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-6 py-5">
            {!codexSetupReady ? (
              <CodexSharedRuntimePanel enabled={open} />
            ) : (
              <>
                <div className="rounded-md border border-emerald-500/30 bg-emerald-500/5 px-3 py-2 text-sm">
                  Connected through the Codex shared runtime
                  <span className="ml-2 font-mono text-xs text-muted-foreground">
                    {sharedRuntimeQuery.data?.url}
                  </span>
                </div>

                <div className="space-y-1.5">
                  <label
                    className="text-sm font-medium"
                    htmlFor="codex-task-search"
                  >
                    Find task
                  </label>
                  <Input
                    id="codex-task-search"
                    onChange={(event) => setSearch(event.target.value)}
                    placeholder="Search by task name, workspace, or UUID"
                    value={search}
                  />
                </div>

                <div className="space-y-1.5">
                  <label
                    className="text-sm font-medium"
                    htmlFor="codex-channel-id"
                  >
                    Channel (optional)
                  </label>
                  <select
                    className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
                    disabled={
                      channelsQuery.isLoading || attachMutation.isPending
                    }
                    id="codex-channel-id"
                    onChange={(event) => setChannelId(event.target.value)}
                    value={channelId}
                  >
                    <option value="">Do not add to a channel yet</option>
                    {channels.map((channel) => (
                      <option key={channel.id} value={channel.id}>
                        #{channel.name}
                      </option>
                    ))}
                  </select>
                </div>

                <div className="space-y-1.5">
                  <label
                    className="text-sm font-medium"
                    htmlFor="codex-task-id"
                  >
                    Codex task
                  </label>
                  <div
                    aria-label="Codex task"
                    className="max-h-64 overflow-y-auto rounded-md border border-input bg-background shadow-xs"
                    id="codex-task-id"
                    role="listbox"
                  >
                    {filteredTasks.map((task) => {
                      const selected = task.id === taskId;
                      return (
                        <button
                          aria-selected={selected}
                          className={`flex w-full items-start gap-3 border-b border-border/50 px-3 py-2.5 text-left last:border-b-0 hover:bg-muted/50 ${
                            selected ? "bg-primary/10" : ""
                          }`}
                          disabled={
                            tasksQuery.isLoading || createMutation.isPending
                          }
                          key={task.id}
                          onClick={() => selectTask(task.id)}
                          role="option"
                          type="button"
                        >
                          <span className="min-w-0 flex-1">
                            <span className="flex items-center gap-2">
                              <span className="truncate text-sm font-medium">
                                {taskLabel(task)}
                              </span>
                              {task.archived ? (
                                <span className="shrink-0 text-xs text-muted-foreground">
                                  Archived
                                </span>
                              ) : null}
                            </span>
                            <span
                              className="mt-0.5 block truncate text-xs text-muted-foreground"
                              title={task.workspace}
                            >
                              {task.workspace}
                            </span>
                            {task.model ? (
                              <span className="mt-0.5 block truncate font-mono text-xs text-muted-foreground">
                                {task.model}
                              </span>
                            ) : null}
                          </span>
                          <span className="shrink-0 pt-0.5 font-mono text-xs text-muted-foreground">
                            {task.id.slice(0, 8)}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                  {tasksQuery.isLoading ? (
                    <p className="text-xs text-muted-foreground">
                      Loading Codex tasks...
                    </p>
                  ) : filteredTasks.length === 0 ? (
                    <p className="text-xs text-muted-foreground">
                      No matching tasks.
                    </p>
                  ) : null}
                </div>

                {selectedTask ? (
                  <div className="space-y-1 text-xs text-muted-foreground">
                    <p className="break-all">{selectedTask.workspace}</p>
                    <p className="font-mono">{selectedTask.id}</p>
                    <p>Codex model: {selectedTask.model ?? "Not recorded"}</p>
                  </div>
                ) : null}

                <div className="space-y-1.5">
                  <label
                    className="text-sm font-medium"
                    htmlFor="codex-agent-name"
                  >
                    Agent name
                  </label>
                  <Input
                    id="codex-agent-name"
                    maxLength={80}
                    onChange={(event) => setName(event.target.value)}
                    value={name}
                  />
                </div>

                <p className="text-sm leading-6 text-muted-foreground">
                  The agent is created offline and connects this task to Buzz
                  through the computer shared runtime. Task history and
                  workspace files stay on this computer.
                </p>

                {!runtimesQuery.isLoading && !codexRuntime ? (
                  <p className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                    The Codex ACP adapter is unavailable. Install or repair
                    Codex in Agent defaults before creating this identity.
                  </p>
                ) : null}
                {error ? (
                  <p className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                    {error.message}
                  </p>
                ) : null}
              </>
            )}
          </div>

          <div className="flex shrink-0 justify-end gap-2 border-t border-border/60 px-6 py-4">
            <Button
              onClick={() => handleOpenChange(false)}
              size="sm"
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
            <Button
              disabled={
                !codexSetupReady ||
                !selectedTask ||
                !codexRuntime ||
                !name.trim() ||
                createMutation.isPending ||
                attachMutation.isPending
              }
              onClick={() => void handleSubmit()}
              size="sm"
              type="button"
            >
              {createMutation.isPending || attachMutation.isPending
                ? "Creating..."
                : "Create agent"}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
