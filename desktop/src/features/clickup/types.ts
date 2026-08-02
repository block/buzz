export type ClickUpUser = {
  id: number;
  username: string;
  email: string | null;
  color: string | null;
  profilePicture: string | null;
  initials: string | null;
};

export type ClickUpConnection = {
  connected: boolean;
  account: ClickUpUser | null;
};

export type ClickUpWorkspace = {
  id: string;
  name: string;
  color: string | null;
  avatar: string | null;
};

export type ClickUpTaskStatus = {
  status: string;
  color: string | null;
  kind: string | null;
};

export type ClickUpTaskPriority = {
  priority: string;
  color: string | null;
};

export type ClickUpTaskLocation = {
  id: string;
  name: string;
};

export type ClickUpCustomField = {
  id: string;
  name: string;
  type: string;
  value: unknown;
};

export type ClickUpDependency = {
  taskId: string | null;
  dependsOn: string | null;
  dependencyOf: string | null;
};

export type ClickUpTask = {
  id: string;
  name: string;
  textContent: string;
  description: string;
  status: ClickUpTaskStatus;
  priority: ClickUpTaskPriority | null;
  dueDateMs: string | null;
  startDateMs: string | null;
  dateCreatedMs: string | null;
  dateUpdatedMs: string | null;
  archived: boolean;
  parentId: string | null;
  url: string;
  workspaceId: string;
  list: ClickUpTaskLocation | null;
  folder: ClickUpTaskLocation | null;
  space: ClickUpTaskLocation | null;
  assignees: ClickUpUser[];
  tags: Array<{
    name: string;
    foreground: string | null;
    background: string | null;
  }>;
  subtasks: ClickUpTask[];
  customFields: ClickUpCustomField[];
  dependencies: ClickUpDependency[];
};

export type ClickUpTaskPage = {
  tasks: ClickUpTask[];
  fetchedAtMs: number;
  truncated: boolean;
};

export type ClickUpComment = {
  id: string;
  text: string;
  dateMs: string | null;
  user: ClickUpUser | null;
  resolved: boolean;
};

export type ClickUpComments = {
  comments: ClickUpComment[];
};

export type ClickUpErrorCode =
  | "forbidden"
  | "identity_changed"
  | "invalid_request"
  | "invalid_token"
  | "keyring_unavailable"
  | "network"
  | "not_connected"
  | "rate_limited"
  | "redirect_rejected"
  | "response_too_large"
  | "server"
  | "unauthorized"
  | "unknown";

export class ClickUpApiError extends Error {
  readonly code: ClickUpErrorCode;
  readonly retryAtMs: number | null;

  constructor(
    code: ClickUpErrorCode,
    message: string,
    retryAtMs: number | null = null,
  ) {
    super(message);
    this.name = "ClickUpApiError";
    this.code = code;
    this.retryAtMs = retryAtMs;
  }
}
