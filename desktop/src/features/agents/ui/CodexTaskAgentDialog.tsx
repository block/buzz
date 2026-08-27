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
import {
  connectCodexSsh,
  listCodexSshConfigHosts,
  listCodexSshTasks,
  stopCodexSsh,
} from "@/shared/api/codexTasks";
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
import { CodexSshDisclosure } from "./CodexSshDisclosure";

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
  const tasksQuery = useCodexTasksQuery({
    enabled: open,
  });
  const channelsQuery = useChannelsQuery({ enabled: open });
  const createMutation = useCreateManagedAgentMutation();
  const attachMutation = useAttachManagedAgentToChannelMutation(null);
  const [search, setSearch] = React.useState("");
  const [taskId, setTaskId] = React.useState("");
  const [name, setName] = React.useState("");
  const [channelId, setChannelId] = React.useState("");
  const [sshHost, setSshHost] = React.useState("");
  const [sshExpanded, setSshExpanded] = React.useState(false);
  const [sshConfigAlias, setSshConfigAlias] = React.useState("");
  const [sshConfigHosts, setSshConfigHosts] = React.useState<
    Awaited<ReturnType<typeof listCodexSshConfigHosts>>
  >([]);
  const [sshPort, setSshPort] = React.useState("22");
  const [sshUser, setSshUser] = React.useState("");
  const [sshIdentity, setSshIdentity] = React.useState("");
  const [sshShell, setSshShell] = React.useState<"posix" | "powershell">(
    "posix",
  );
  const [sshRuntime, setSshRuntime] = React.useState<Awaited<
    ReturnType<typeof connectCodexSsh>
  > | null>(null);
  const [sshPending, setSshPending] = React.useState(false);
  const [sshError, setSshError] = React.useState<string | null>(null);
  const [remoteTasks, setRemoteTasks] = React.useState<CodexTaskSummary[]>([]);
  const [remoteTasksPending, setRemoteTasksPending] = React.useState(false);
  const [remoteTaskId, setRemoteTaskId] = React.useState<string | null>(null);
  const codexSetupReady = Boolean(codexRuntime);

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
  const selectedRemoteTask =
    remoteTasks.find((task) => task.id === remoteTaskId) ?? null;
  const selectedTaskForSubmit = selectedRemoteTask ?? selectedTask;
  const selectedTaskRuntimeReady = selectedRemoteTask
    ? sshRuntime !== null
    : sharedRuntimeReady;
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
    if (!open || taskId || remoteTaskId || tasks.length === 0) return;
    setTaskId(tasks[0].id);
    setName(taskLabel(tasks[0]));
  }, [open, remoteTaskId, taskId, tasks]);

  React.useEffect(() => {
    if (!open || !sshExpanded) return;
    void listCodexSshConfigHosts()
      .then(setSshConfigHosts)
      .catch((cause) =>
        setSshError(cause instanceof Error ? cause.message : String(cause)),
      );
  }, [open, sshExpanded]);

  React.useEffect(() => {
    // Selecting a remote task intentionally clears the local task id. Do not
    // interpret that empty id as "nothing selected" and immediately restore
    // the first local task on the next render.
    if (!open || remoteTaskId || filteredTasks.length === 0) return;

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
  }, [filteredTasks, open, remoteTaskId, search, taskId, tasks]);

  function reset() {
    setSearch("");
    setTaskId("");
    setName("");
    setChannelId("");
    setSshExpanded(false);
    setSshConfigAlias("");
    setSshRuntime(null);
    setSshError(null);
    setRemoteTasks([]);
    setRemoteTaskId(null);
    createMutation.reset();
    attachMutation.reset();
  }

  function selectSshConfigHost(alias: string) {
    setSshConfigAlias(alias);
    const profile = sshConfigHosts.find((host) => host.alias === alias);
    if (!profile) return;
    setSshHost(profile.alias);
    setSshUser(profile.username);
    setSshPort(String(profile.port));
    setSshIdentity("");
    setSshRuntime(null);
    setRemoteTasks([]);
    setRemoteTaskId(null);
    setSshError(null);
  }

  async function handleSshConnect() {
    setSshPending(true);
    setSshError(null);
    try {
      setSshRuntime(
        await connectCodexSsh({
          host: sshHost,
          port: Number(sshPort) || 22,
          username: sshUser,
          identityFile: sshIdentity,
          remoteShell: sshShell,
        }),
      );
    } catch (cause) {
      setSshError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSshPending(false);
    }
  }

  async function handleSshDisconnect() {
    if (!sshRuntime) return;
    await stopCodexSsh({
      host: sshRuntime.host,
      username: sshRuntime.username,
      port: sshRuntime.port,
    });
    setSshRuntime(null);
    setRemoteTasks([]);
    setRemoteTaskId(null);
  }

  async function handleLoadRemoteTasks() {
    setRemoteTasksPending(true);
    setSshError(null);
    try {
      setRemoteTasks(
        await listCodexSshTasks({
          host: sshHost,
          port: Number(sshPort) || 22,
          username: sshUser,
          identityFile: sshIdentity,
          remoteShell: sshShell,
        }),
      );
    } catch (cause) {
      setSshError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setRemoteTasksPending(false);
    }
  }

  function handleOpenChange(next: boolean) {
    if (!next) {
      if (sshRuntime) {
        void stopCodexSsh({
          host: sshRuntime.host,
          username: sshRuntime.username,
          port: sshRuntime.port,
        });
      }
      reset();
    }
    onOpenChange(next);
  }

  function selectTask(nextTaskId: string) {
    setRemoteTaskId(null);
    setTaskId(nextTaskId);
    const task = tasks.find((candidate) => candidate.id === nextTaskId);
    if (task) setName(taskLabel(task));
  }

  function selectRemoteTask(nextTask: CodexTaskSummary) {
    setTaskId("");
    setRemoteTaskId(nextTask.id);
    setName(taskLabel(nextTask));
  }

  async function handleSubmit() {
    const task = selectedTaskForSubmit;
    if (!task || !selectedTaskRuntimeReady || !codexRuntime || !name.trim())
      return;
    try {
      const created = await createMutation.mutateAsync({
        name: name.trim(),
        codexTaskId: task.id,
        codexAppServerUrl: selectedRemoteTask
          ? undefined
          : sharedRuntimeQuery.data?.url,
        codexSshHost: selectedRemoteTask ? sshHost : undefined,
        codexSshPort: selectedRemoteTask ? Number(sshPort) || 22 : undefined,
        codexSshUsername: selectedRemoteTask ? sshUser : undefined,
        codexSshIdentityFile: selectedRemoteTask ? sshIdentity : undefined,
        codexSshRemoteAppServerPort: selectedRemoteTask ? 51919 : undefined,
        codexSshRemoteShell: selectedRemoteTask ? sshShell : undefined,
        codexTaskName: task.threadName,
        codexTaskWorkspace: task.workspace,
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
      <DialogContent className="w-[min(92vw,48rem)] max-w-none overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="shrink-0 border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>Add a Codex task as an agent</DialogTitle>
            <DialogDescription className="max-w-full">
              Create an independent Buzz identity that resumes one existing
              Codex task across every Channel it joins.
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 min-w-0 flex-1 space-y-5 overflow-y-auto overflow-x-hidden px-6 py-5">
            {!codexSetupReady ? (
              <CodexSharedRuntimePanel enabled={open} />
            ) : (
              <>
                {!sharedRuntimeReady ? (
                  <CodexSharedRuntimePanel enabled={open} />
                ) : null}
                <CodexSshDisclosure
                  connected={sshRuntime !== null}
                  expanded={sshExpanded}
                  onExpandedChange={setSshExpanded}
                >
                  <p className="text-xs text-muted-foreground">
                    Buzz starts <code>codex app-server</code> on the remote
                    computer through SSH and creates a local tunnel; private key
                    contents never leave your computer.
                  </p>
                  <select
                    aria-label="SSH config host"
                    className="h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    disabled={sshRuntime !== null}
                    onChange={(e) => selectSshConfigHost(e.target.value)}
                    value={sshConfigAlias}
                  >
                    <option value="">Select from ~/.ssh/config</option>
                    {sshConfigHosts.map((host) => (
                      <option key={host.alias} value={host.alias}>
                        {host.alias} -{" "}
                        {host.username ? `${host.username}@` : ""}
                        {host.hostname}:{host.port}
                      </option>
                    ))}
                  </select>
                  <div className="grid grid-cols-2 gap-2">
                    <Input
                      aria-label="SSH host"
                      disabled={sshRuntime !== null}
                      onChange={(e) => setSshHost(e.target.value)}
                      placeholder="host"
                      value={sshHost}
                    />
                    <Input
                      aria-label="SSH port"
                      disabled={sshRuntime !== null}
                      onChange={(e) => setSshPort(e.target.value)}
                      placeholder="22"
                      value={sshPort}
                    />
                    <Input
                      aria-label="SSH username"
                      disabled={sshRuntime !== null}
                      onChange={(e) => setSshUser(e.target.value)}
                      placeholder="username"
                      value={sshUser}
                    />
                    <Input
                      aria-label="SSH identity file"
                      disabled={sshRuntime !== null}
                      onChange={(e) => setSshIdentity(e.target.value)}
                      placeholder="Optional: ~/.ssh/config or default key"
                      value={sshIdentity}
                    />
                    <select
                      aria-label="Remote shell"
                      className="h-9 rounded-md border border-input bg-background px-3 py-2 text-sm"
                      disabled={sshRuntime !== null}
                      onChange={(e) =>
                        setSshShell(e.target.value as "posix" | "powershell")
                      }
                      value={sshShell}
                    >
                      <option value="posix">macOS / Linux</option>
                      <option value="powershell">Windows PowerShell</option>
                    </select>
                  </div>
                  <div className="flex items-center gap-2">
                    {sshRuntime ? (
                      <Button
                        onClick={() => void handleSshDisconnect()}
                        size="sm"
                        type="button"
                        variant="outline"
                      >
                        Disconnect
                      </Button>
                    ) : (
                      <Button
                        disabled={sshPending || !sshHost || !sshUser}
                        onClick={() => void handleSshConnect()}
                        size="sm"
                        type="button"
                        variant="outline"
                      >
                        {sshPending ? "Connecting..." : "Connect SSH"}
                      </Button>
                    )}
                    {sshRuntime ? (
                      <span className="break-all text-xs text-emerald-600">
                        Tunnel ready: {sshRuntime.appServerUrl}
                      </span>
                    ) : null}
                  </div>
                  <div className="flex items-center gap-2">
                    <Button
                      disabled={remoteTasksPending || sshRuntime === null}
                      onClick={() => void handleLoadRemoteTasks()}
                      size="sm"
                      type="button"
                      variant="outline"
                    >
                      {remoteTasksPending
                        ? "Loading remote tasks..."
                        : "Load remote tasks"}
                    </Button>
                    {remoteTasks.length > 0 ? (
                      <span className="text-xs text-muted-foreground">
                        {remoteTasks.length} remote tasks found
                      </span>
                    ) : null}
                  </div>
                  {remoteTasks.length > 0 ? (
                    <div className="max-h-40 overflow-y-auto rounded-md border border-input text-xs">
                      {remoteTasks.map((task) => (
                        <button
                          className={`block w-full border-b border-border/50 px-3 py-2 text-left last:border-b-0 ${remoteTaskId === task.id ? "bg-primary/10" : ""}`}
                          key={task.id}
                          onClick={() => {
                            selectRemoteTask(task);
                          }}
                          type="button"
                        >
                          <div className="font-medium">{taskLabel(task)}</div>
                          <div className="font-mono text-muted-foreground">
                            {task.id}
                          </div>
                        </button>
                      ))}
                    </div>
                  ) : null}
                  {sshError ? (
                    <p className="text-xs text-destructive">{sshError}</p>
                  ) : null}
                </CodexSshDisclosure>
                {sharedRuntimeReady ? (
                  <div className="rounded-md border border-emerald-500/30 bg-emerald-500/5 px-3 py-2 text-sm leading-5">
                    <span>Connected through the Codex shared runtime</span>
                    <span className="ml-2 break-all font-mono text-xs text-muted-foreground">
                      {sharedRuntimeQuery.data?.url}
                    </span>
                  </div>
                ) : null}
                {sshRuntime ? (
                  <p className="rounded-md border border-emerald-500/30 bg-emerald-500/5 px-3 py-2 text-sm leading-5 text-emerald-800 dark:text-emerald-200">
                    SSH runtime is connected. Select a remote task below to
                    create an agent that runs on the remote Codex computer.
                  </p>
                ) : null}

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
                          className={`grid w-full grid-cols-[minmax(0,1fr)_auto] items-start gap-3 border-b border-border/50 px-3 py-2.5 text-left last:border-b-0 hover:bg-muted/50 ${
                            selected ? "bg-primary/10" : ""
                          }`}
                          disabled={
                            tasksQuery.isLoading ||
                            createMutation.isPending ||
                            Boolean(sshRuntime)
                          }
                          key={task.id}
                          onClick={() => selectTask(task.id)}
                          role="option"
                          type="button"
                        >
                          <span className="min-w-0 flex-1">
                            <span className="flex min-w-0 items-center gap-2">
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
                          <span className="max-w-24 truncate pt-0.5 font-mono text-xs text-muted-foreground">
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
                  <div className="min-w-0 space-y-1 text-xs text-muted-foreground">
                    <p className="break-all">{selectedTask.workspace}</p>
                    <p className="break-all font-mono">{selectedTask.id}</p>
                    <p className="break-words">
                      Codex model: {selectedTask.model ?? "Not recorded"}
                    </p>
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

                <p className="break-words text-sm leading-6 text-muted-foreground">
                  {remoteTaskId
                    ? "The agent connects to the selected remote Codex task through SSH. Task history and workspace files remain on the remote computer."
                    : "The agent connects this task to Buzz through the computer shared runtime. Task history and workspace files stay on this computer."}
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
                !selectedTaskForSubmit ||
                !selectedTaskRuntimeReady ||
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
