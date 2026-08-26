export type CodexTaskBinding = {
  taskId: string;
  threadName: string;
  workspace: string;
  updatedAt: string;
  model: string | null;
  appServerUrl: string | null;
  sshHost: string | null;
  sshPort: number | null;
  sshUsername: string | null;
  sshIdentityFile: string | null;
};

export type CodexTaskSummary = {
  id: string;
  threadName: string;
  workspace: string;
  updatedAt: string;
  archived: boolean;
  model: string | null;
};

export type CodexTaskHistoryMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  timestamp: string | null;
};

export type CodexTaskHistory = {
  taskId: string;
  threadName: string;
  messages: CodexTaskHistoryMessage[];
  truncated: boolean;
};

export type CodexSharedRuntimeState =
  | "setup_required"
  | "ready"
  | "unavailable";

export type CodexSharedRuntimeStatus = {
  enabled: boolean;
  state: CodexSharedRuntimeState;
  url: string;
  detail: string | null;
  desktopProcessIds: number[];
  privateAppServerProcessIds: number[];
  desktopDetectionError: string | null;
};

export type CodexSshRuntimeStatus = {
  host: string;
  port: number;
  username: string;
  localPort: number;
  appServerUrl: string;
};

export type CodexSshConfigHost = {
  alias: string;
  hostname: string;
  username: string;
  port: number;
};

export type RawCodexSharedRuntimeStatus = {
  enabled: boolean;
  state: CodexSharedRuntimeState;
  url: string;
  detail?: string | null;
  desktop_process_ids?: number[];
  private_app_server_process_ids?: number[];
  desktop_detection_error?: string | null;
};

export function fromRawCodexSharedRuntimeStatus(
  status: RawCodexSharedRuntimeStatus,
): CodexSharedRuntimeStatus {
  return {
    enabled: status.enabled,
    state: status.state,
    url: status.url,
    detail: status.detail ?? null,
    desktopProcessIds: status.desktop_process_ids ?? [],
    privateAppServerProcessIds: status.private_app_server_process_ids ?? [],
    desktopDetectionError: status.desktop_detection_error ?? null,
  };
}

export type RawCodexTaskBinding = {
  task_id: string;
  thread_name: string;
  workspace: string;
  updated_at: string;
  model?: string | null;
  app_server_url?: string | null;
  ssh_host?: string | null;
  ssh_port?: number | null;
  ssh_username?: string | null;
  ssh_identity_file?: string | null;
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
        appServerUrl: binding.app_server_url ?? null,
        sshHost: binding.ssh_host ?? null,
        sshPort: binding.ssh_port ?? null,
        sshUsername: binding.ssh_username ?? null,
        sshIdentityFile: binding.ssh_identity_file ?? null,
      }
    : null;
}
