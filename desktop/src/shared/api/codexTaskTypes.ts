export type CodexTaskBinding = {
  taskId: string;
  threadName: string;
  workspace: string;
  updatedAt: string;
  model: string | null;
};

export type CodexTaskSummary = {
  id: string;
  threadName: string;
  workspace: string;
  updatedAt: string;
  archived: boolean;
  model: string | null;
};

export type RawCodexTaskBinding = {
  task_id: string;
  thread_name: string;
  workspace: string;
  updated_at: string;
  model?: string | null;
};

export function fromRawCodexTaskBinding(
  binding?: RawCodexTaskBinding | null,
): CodexTaskBinding | null {
  return binding
    ? {
        taskId: binding.task_id,
        threadName: binding.thread_name,
        workspace: binding.workspace,
        updatedAt: binding.updated_at,
        model: binding.model ?? null,
      }
    : null;
}
