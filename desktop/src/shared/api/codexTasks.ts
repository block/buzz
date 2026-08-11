import type { CodexTaskSummary } from "@/shared/api/codexTaskTypes";
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
