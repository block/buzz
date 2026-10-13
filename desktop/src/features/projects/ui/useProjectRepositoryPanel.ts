import * as React from "react";

import {
  type ProjectDetailAgentContext,
  withProjectSelectionAgentContext,
} from "@/features/projects/lib/projectDetailAgentContext";
import type { ProjectSelectionItem } from "@/features/projects/lib/projectSelection";
import { isPhoneLayout } from "@/shared/lib/phoneLayout";
import type { ProjectRightPanelMode } from "./ProjectRightPanelControls";

const REPOSITORY_PANEL_COLLAPSED_KEY =
  "buzz.desktop.project-repository-panel-collapsed";
const PROJECT_RIGHT_PANEL_MODE_KEY = "buzz.desktop.project-right-panel-mode";

function initialCollapsed() {
  // Phone: a project always opens on the chat/README side; the repo is a full-screen toggle, never a split.
  if (isPhoneLayout()) return true;
  try {
    return (
      globalThis.sessionStorage?.getItem(REPOSITORY_PANEL_COLLAPSED_KEY) ===
      "true"
    );
  } catch {
    return false;
  }
}

function initialMode(): ProjectRightPanelMode {
  try {
    return globalThis.sessionStorage?.getItem(PROJECT_RIGHT_PANEL_MODE_KEY) ===
      "chat"
      ? "chat"
      : "repository";
  } catch {
    return "repository";
  }
}

export function useProjectRepositoryPanel(ownerKey: string) {
  const [collapsed, setCollapsedState] = React.useState(initialCollapsed);
  const [selectionAgentContext, setSelectionAgentContext] = React.useState<{
    context: ProjectDetailAgentContext;
    ownerKey: string;
  } | null>(null);
  const [mode, setModeState] =
    React.useState<ProjectRightPanelMode>(initialMode);
  const setCollapsed = React.useCallback((nextCollapsed: boolean) => {
    setCollapsedState(nextCollapsed);
    try {
      globalThis.sessionStorage?.setItem(
        REPOSITORY_PANEL_COLLAPSED_KEY,
        String(nextCollapsed),
      );
    } catch {
      // Persistence is best-effort; the in-memory drawer state still works.
    }
  }, []);
  const setMode = React.useCallback((nextMode: ProjectRightPanelMode) => {
    setSelectionAgentContext(null);
    setModeState(nextMode);
    try {
      globalThis.sessionStorage?.setItem(
        PROJECT_RIGHT_PANEL_MODE_KEY,
        nextMode,
      );
    } catch {
      // Persistence is best-effort; the in-memory panel mode still works.
    }
  }, []);
  React.useEffect(() => {
    setSelectionAgentContext((current) =>
      current?.ownerKey === ownerKey ? current : null,
    );
  }, [ownerKey]);
  const openSelectionChat = React.useCallback(
    (context: ProjectDetailAgentContext, items: ProjectSelectionItem[]) => {
      setMode("chat");
      setSelectionAgentContext({
        context: withProjectSelectionAgentContext(context, items),
        ownerKey,
      });
      setCollapsed(false);
    },
    [ownerKey, setCollapsed, setMode],
  );

  return {
    agentContext: (fallback: ProjectDetailAgentContext) =>
      selectionAgentContext?.ownerKey === ownerKey
        ? selectionAgentContext.context
        : fallback,
    collapse: React.useCallback(() => setCollapsed(true), [setCollapsed]),
    collapsed,
    expand: React.useCallback(() => setCollapsed(false), [setCollapsed]),
    mode,
    openSelectionChat,
    openSelectionChatFor: (context: ProjectDetailAgentContext) =>
      openSelectionChat.bind(null, context),
    setMode,
  };
}
