import type { AppNavigationTarget } from "@/app/navigation/NavigationTargetContext";

export type PulseWorkspacePage = "projects" | "agents" | "workflows";
export type PulseView = "all" | "search" | "conversation" | PulseWorkspacePage;

export function isPulseWorkspacePage(
  value: unknown,
): value is PulseWorkspacePage {
  return value === "projects" || value === "agents" || value === "workflows";
}

export const PULSE_WORKSPACE_KEYS = [
  "projectId",
  "projectSection",
  "workflowId",
  "view",
  "pane",
  "profilePersona",
  "commitHash",
  "filePath",
  "pullRequestId",
  "issueId",
  "repositoryId",
  "tab",
] as const;
export const CLEAR_WORKSPACE_PANELS = Object.fromEntries(
  PULSE_WORKSPACE_KEYS.map((key) => [key, null]),
) as Record<(typeof PULSE_WORKSPACE_KEYS)[number], null>;

/** Reuse feature navigation while retaining Pulse's rail and selected variation. */
export function workspaceNavigationTarget(
  target: AppNavigationTarget,
  layout: string | null,
  conversation: string | null,
): AppNavigationTarget {
  let feed: PulseView;
  let params: Record<string, string | undefined> = {};
  switch (target.to) {
    case "/projects":
      feed = "projects";
      break;
    case "/projects/$projectId":
      feed = "projects";
      params = { projectId: target.params?.projectId };
      break;
    case "/agents":
      feed = "agents";
      break;
    case "/workflows":
      feed = "workflows";
      break;
    case "/workflows/$workflowId":
      feed = "workflows";
      params = { workflowId: target.params?.workflowId };
      break;
    case "/channels/$channelId":
      // Special navigation (auto-send, targeted history) retains its canonical route.
      if (target.search?.autoSend || target.search?.messageId) return target;
      feed = "conversation";
      params = { conversation: target.params?.channelId };
      break;
    default:
      return target;
  }
  return {
    ...target,
    to: "/pulse",
    params: undefined,
    search: {
      layout: layout ?? undefined,
      conversation: conversation ?? undefined,
      ...target.search,
      feed,
      ...params,
    },
  };
}
