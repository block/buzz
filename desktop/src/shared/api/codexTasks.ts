import type {
  RawCodexSharedRuntimeStatus,
  CodexSharedRuntimeStatus,
  CodexTaskSummary,
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

export async function listCodexTasks(): Promise<CodexTaskSummary[]> {
  const tasks = await invokeTauri<RawCodexTaskSummary[]>("list_codex_tasks");
  return tasks.map((task) => ({
    id: task.id,
    threadName: task.thread_name,
    workspace: task.workspace,
    updatedAt: task.updated_at,
    archived: task.archived,
    model: task.model ?? null,
  }));
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
