import type {
  RawCodexSharedRuntimeStatus,
  CodexSharedRuntimeStatus,
  CodexTaskHistory,
  CodexTaskSummary,
  CodexSshRuntimeStatus,
  CodexSshConfigHost,
} from "@/shared/api/codexTaskTypes";
import { fromRawCodexSharedRuntimeStatus } from "@/shared/api/codexTaskTypes";
import { invokeTauri } from "@/shared/api/tauri";

type RawCodexTaskSummary = {
  id: string;
  thread_name: string;
  workspace: string;
  updated_at: string;
  archived: boolean;
  model?: string | null;
};

function fromRawCodexTaskSummary(task: RawCodexTaskSummary): CodexTaskSummary {
  return {
    id: task.id,
    threadName: task.thread_name,
    workspace: task.workspace,
    updatedAt: task.updated_at,
    archived: task.archived,
    model: task.model ?? null,
  };
}

export async function listCodexTasks(): Promise<CodexTaskSummary[]> {
  const tasks = await invokeTauri<RawCodexTaskSummary[]>("list_codex_tasks");
  return tasks.map(fromRawCodexTaskSummary);
}

type RawCodexTaskHistory = {
  task_id: string;
  thread_name: string;
  messages: Array<{
    id: string;
    role: "user" | "assistant";
    content: string;
    timestamp?: string | null;
  }>;
  truncated: boolean;
};

export async function getCodexTaskHistory(
  agentPubkey: string,
): Promise<CodexTaskHistory> {
  const history = await invokeTauri<RawCodexTaskHistory>(
    "get_codex_task_history",
    { agentPubkey },
  );
  return {
    taskId: history.task_id,
    threadName: history.thread_name,
    messages: history.messages.map((message) => ({
      id: message.id,
      role: message.role,
      content: message.content,
      timestamp: message.timestamp ?? null,
    })),
    truncated: history.truncated,
  };
}

async function invokeCodexSharedRuntimeStatus(
  command: string,
  args?: Record<string, unknown>,
): Promise<CodexSharedRuntimeStatus> {
  const status = await invokeTauri<RawCodexSharedRuntimeStatus>(command, args);
  return fromRawCodexSharedRuntimeStatus(status);
}

export function getCodexSharedRuntimeStatus() {
  return invokeCodexSharedRuntimeStatus("get_codex_shared_runtime_status");
}

export function enableCodexSharedRuntime() {
  return invokeCodexSharedRuntimeStatus("enable_codex_shared_runtime");
}

export function launchCodexDesktopShared() {
  return invokeTauri<void>("launch_codex_desktop_shared");
}

export function takeOverCodexDesktopShared() {
  return invokeCodexSharedRuntimeStatus("take_over_codex_desktop_shared", {
    confirmed: true,
  });
}

export function connectCodexSsh(input: {
  host: string;
  port?: number;
  username: string;
  identityFile?: string;
  remoteAppServerPort?: number;
  remoteShell?: "posix" | "powershell";
}) {
  const request = { ...input };
  if (request.identityFile) {
    request.identityFile = request.identityFile.trim();
    if (!request.identityFile) delete request.identityFile;
  }
  return invokeTauri<CodexSshRuntimeStatus>("connect_codex_ssh", { request });
}

export function listCodexSshConfigHosts() {
  return invokeTauri<CodexSshConfigHost[]>("list_codex_ssh_config_hosts");
}

export function stopCodexSsh(input: {
  host: string;
  username: string;
  port: number;
}) {
  return invokeTauri<void>("stop_codex_ssh", input);
}

export function listCodexSshTasks(input: {
  host: string;
  port?: number;
  username: string;
  identityFile?: string;
  remoteShell?: "posix" | "powershell";
}) {
  const request = { ...input };
  if (request.identityFile) {
    request.identityFile = request.identityFile.trim();
    if (!request.identityFile) delete request.identityFile;
  }
  return invokeTauri<RawCodexTaskSummary[]>("list_codex_ssh_tasks", {
    request,
  }).then((tasks) => tasks.map(fromRawCodexTaskSummary));
}
