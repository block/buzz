/**
 * REG-15: typed desktop client for the relay's channel-task REST API.
 *
 * Thin wrapper over the four Tauri commands (which sign each request with the
 * shared NIP-98 builder and honor the active workspace relay). "Channel task"
 * naming is deliberate (work/REG-15/reflecting.md D4 Option A): Projects'
 * kind:30617 issues are a different substrate and must stay visually distinct.
 */

/** camelCase mirror of `ChannelTask` on the Rust side. */
export type ChannelTask = {
  id: string;
  channelId: string | null;
  title: string;
  status: string;
  assignee: string | null;
  createdBy: string | null;
  updatedAt: number;
};

/** One source community's outcome in the My-Tasks fan-in. */
export type ChannelTaskSource = {
  relayBase: string;
  tasks: ChannelTask[];
  error: string | null;
};

export type TaskStatusFilter = "open" | "in_progress" | "done";

/** Relay statuses we render specially; unknown values degrade to a badge. */
export const OPEN_STATUSES = new Set(["open", "todo"]);
export const DONE_STATUSES = new Set(["done", "completed"]);

export function isTaskDone(task: ChannelTask): boolean {
  return DONE_STATUSES.has(task.status);
}

/**
 * Sort newest-updated first (the relay already orders within a community, but
 * the My-Tasks fan-in merges sources and must re-order the merged list).
 */
export function byUpdatedAtDesc(a: ChannelTask, b: ChannelTask): number {
  return b.updatedAt - a.updatedAt;
}

export async function listChannelTasks(input: {
  channelId?: string | null;
  status?: TaskStatusFilter;
  limit?: number;
}): Promise<ChannelTask[]> {
  const { invokeTauri } = await import("@/shared/api/tauri");
  return invokeTauri<ChannelTask[]>("tasks_list", {
    channelId: input.channelId ?? null,
    status: input.status ?? null,
    limit: input.limit ?? null,
  });
}

export async function createChannelTask(input: {
  title: string;
  channelId?: string | null;
  body?: string | null;
}): Promise<ChannelTask> {
  const { invokeTauri } = await import("@/shared/api/tauri");
  return invokeTauri<ChannelTask>("tasks_create", {
    title: input.title,
    channelId: input.channelId ?? null,
    bodyText: input.body ?? null,
  });
}

export async function setChannelTaskStatus(
  taskId: string,
  status: "open" | "in_progress" | "done",
): Promise<ChannelTask> {
  const { invokeTauri } = await import("@/shared/api/tauri");
  return invokeTauri<ChannelTask>("tasks_set_status", {
    taskId,
    status,
  });
}

/**
 * REG-16: assign or clear a task's owner.
 *
 * `assignee: null` UNASSIGNS — the relay's PATCH treats the field as doubly
 * optional (absent = leave alone, null = clear), and the Tauri shim emits an
 * explicit JSON null so that distinction survives the wire.
 */
export async function setChannelTaskAssignee(
  taskId: string,
  assignee: string | null,
): Promise<ChannelTask> {
  const { invokeTauri } = await import("@/shared/api/tauri");
  return invokeTauri<ChannelTask>("tasks_set_assignee", {
    taskId,
    assignee,
  });
}

export async function listMyWorkspaceTasks(
  relayBases: string[],
): Promise<ChannelTaskSource[]> {
  const { invokeTauri } = await import("@/shared/api/tauri");
  return invokeTauri<ChannelTaskSource[]>("tasks_my_workspaces", {
    relayBases,
  });
}
