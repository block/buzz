import { invokeTauri } from "@/shared/api/tauri";
import {
  ClickUpApiError,
  type ClickUpComment,
  type ClickUpComments,
  type ClickUpConnection,
  type ClickUpDependency,
  type ClickUpErrorCode,
  type ClickUpTask,
  type ClickUpTaskPage,
  type ClickUpUser,
  type ClickUpWorkspace,
} from "@/features/clickup/types";

type RawClickUpUser = {
  id: number;
  username: string;
  email?: string | null;
  color?: string | null;
  profile_picture?: string | null;
  initials?: string | null;
};

type RawClickUpConnection = {
  connected: boolean;
  account: RawClickUpUser | null;
};

type RawClickUpWorkspace = {
  id: string;
  name: string;
  color?: string | null;
  avatar?: string | null;
};

type RawClickUpTask = {
  id: string;
  name: string;
  text_content?: string;
  description?: string;
  status?: { status?: string; color?: string | null; type?: string | null };
  priority?: { priority?: string; color?: string | null } | null;
  due_date?: string | null;
  start_date?: string | null;
  date_created?: string | null;
  date_updated?: string | null;
  archived?: boolean;
  parent?: string | null;
  url?: string;
  team_id?: string;
  list?: { id?: string; name?: string } | null;
  folder?: { id?: string; name?: string } | null;
  space?: { id?: string; name?: string } | null;
  assignees?: RawClickUpUser[];
  tags?: Array<{
    name?: string;
    tag_fg?: string | null;
    tag_bg?: string | null;
  }>;
  subtasks?: RawClickUpTask[];
  custom_fields?: Array<{
    id?: string;
    name?: string;
    type?: string;
    value?: unknown;
  }>;
  dependencies?: Array<{
    task_id?: string | null;
    depends_on?: string | null;
    dependency_of?: string | null;
  }>;
};

type RawClickUpTaskPage = {
  tasks: RawClickUpTask[];
  fetched_at_ms: number;
  truncated: boolean;
};

type RawClickUpComment = {
  id: string | number;
  comment_text?: string | null;
  comment?: Array<{ text?: string }>;
  date?: string | null;
  user?: RawClickUpUser | null;
  resolved?: boolean;
};

type RawClickUpComments = {
  comments: RawClickUpComment[];
};

const KNOWN_ERROR_CODES = new Set<ClickUpErrorCode>([
  "forbidden",
  "identity_changed",
  "invalid_request",
  "invalid_token",
  "keyring_unavailable",
  "network",
  "not_connected",
  "rate_limited",
  "redirect_rejected",
  "response_too_large",
  "server",
  "unauthorized",
  "unknown",
]);

function mapUser(user: RawClickUpUser): ClickUpUser {
  return {
    id: user.id,
    username: user.username,
    email: user.email ?? null,
    color: user.color ?? null,
    profilePicture: user.profile_picture ?? null,
    initials: user.initials ?? null,
  };
}

function mapLocation(
  location: { id?: string; name?: string } | null | undefined,
) {
  if (!location?.id || !location.name) return null;
  return { id: location.id, name: location.name };
}

function mapDependency(
  dependency: NonNullable<RawClickUpTask["dependencies"]>[number],
): ClickUpDependency {
  return {
    taskId: dependency.task_id ?? null,
    dependsOn: dependency.depends_on ?? null,
    dependencyOf: dependency.dependency_of ?? null,
  };
}

function mapTask(task: RawClickUpTask): ClickUpTask {
  return {
    id: task.id,
    name: task.name,
    textContent: task.text_content ?? "",
    description: task.description ?? "",
    status: {
      status: task.status?.status ?? "Unknown",
      color: task.status?.color ?? null,
      kind: task.status?.type ?? null,
    },
    priority: task.priority?.priority
      ? {
          priority: task.priority.priority,
          color: task.priority.color ?? null,
        }
      : null,
    dueDateMs: task.due_date ?? null,
    startDateMs: task.start_date ?? null,
    dateCreatedMs: task.date_created ?? null,
    dateUpdatedMs: task.date_updated ?? null,
    archived: task.archived ?? false,
    parentId: task.parent ?? null,
    url: task.url ?? "",
    workspaceId: task.team_id ?? "",
    list: mapLocation(task.list),
    folder: mapLocation(task.folder),
    space: mapLocation(task.space),
    assignees: (task.assignees ?? []).map(mapUser),
    tags: (task.tags ?? [])
      .filter((tag) => Boolean(tag.name))
      .map((tag) => ({
        name: tag.name ?? "",
        foreground: tag.tag_fg ?? null,
        background: tag.tag_bg ?? null,
      })),
    subtasks: (task.subtasks ?? []).map(mapTask),
    customFields: (task.custom_fields ?? [])
      .filter((field) => Boolean(field.id && field.name && field.type))
      .map((field) => ({
        id: field.id ?? "",
        name: field.name ?? "",
        type: field.type ?? "",
        value: field.value,
      })),
    dependencies: (task.dependencies ?? []).map(mapDependency),
  };
}

function mapComment(comment: RawClickUpComment): ClickUpComment {
  const parts = (comment.comment ?? [])
    .map((part) => part.text?.trim() ?? "")
    .filter(Boolean);
  return {
    id: String(comment.id),
    text: comment.comment_text?.trim() || parts.join(" "),
    dateMs: comment.date ?? null,
    user: comment.user ? mapUser(comment.user) : null,
    resolved: comment.resolved ?? false,
  };
}

export function toClickUpApiError(error: unknown): ClickUpApiError {
  if (error instanceof ClickUpApiError) return error;
  const message = error instanceof Error ? error.message : String(error);
  const match = /^clickup:([^:]+):([^:]*):(.*)$/s.exec(message);
  if (!match) return new ClickUpApiError("unknown", message);

  const rawCode = match[1] ?? "unknown";
  const code = KNOWN_ERROR_CODES.has(rawCode as ClickUpErrorCode)
    ? (rawCode as ClickUpErrorCode)
    : "unknown";
  const parsedRetryAt = Number(match[2]);
  return new ClickUpApiError(
    code,
    match[3] || "ClickUp request failed.",
    Number.isFinite(parsedRetryAt) && parsedRetryAt > 0 ? parsedRetryAt : null,
  );
}

async function clickUpInvoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invokeTauri<T>(command, args);
  } catch (error) {
    throw toClickUpApiError(error);
  }
}

export async function getClickUpConnection(): Promise<ClickUpConnection> {
  const result = await clickUpInvoke<RawClickUpConnection>(
    "clickup_auth_status",
  );
  return {
    connected: result.connected,
    account: result.account ? mapUser(result.account) : null,
  };
}

export async function connectClickUp(
  personalToken: string,
): Promise<ClickUpConnection> {
  const result = await clickUpInvoke<RawClickUpConnection>("clickup_connect", {
    personalToken,
  });
  return {
    connected: result.connected,
    account: result.account ? mapUser(result.account) : null,
  };
}

export function disconnectClickUp(): Promise<void> {
  return clickUpInvoke<void>("clickup_disconnect");
}

export async function listClickUpWorkspaces(): Promise<ClickUpWorkspace[]> {
  const result = await clickUpInvoke<RawClickUpWorkspace[]>(
    "clickup_list_workspaces",
  );
  return result.map((workspace) => ({
    id: workspace.id,
    name: workspace.name,
    color: workspace.color ?? null,
    avatar: workspace.avatar ?? null,
  }));
}

export async function listClickUpTasks(
  workspaceId: string,
): Promise<ClickUpTaskPage> {
  const result = await clickUpInvoke<RawClickUpTaskPage>("clickup_list_tasks", {
    workspaceId,
  });
  return {
    tasks: result.tasks.map(mapTask),
    fetchedAtMs: result.fetched_at_ms,
    truncated: result.truncated,
  };
}

export async function getClickUpTask(
  workspaceId: string,
  taskId: string,
): Promise<ClickUpTask> {
  return mapTask(
    await clickUpInvoke<RawClickUpTask>("clickup_get_task", {
      workspaceId,
      taskId,
    }),
  );
}

export async function getClickUpTaskComments(
  workspaceId: string,
  taskId: string,
): Promise<ClickUpComments> {
  const result = await clickUpInvoke<RawClickUpComments>(
    "clickup_get_task_comments",
    { workspaceId, taskId },
  );
  return { comments: result.comments.map(mapComment) };
}
