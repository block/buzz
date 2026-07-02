import * as React from "react";

import type { ChatProject } from "@/features/chats/lib/chatSetup";

const STORAGE_PREFIX = "buzz:chat-projects:v1";
const STORAGE_EVENT = "buzz:chat-projects-changed";

function storageKey(workspaceId: string | null | undefined) {
  return `${STORAGE_PREFIX}:${workspaceId ?? "default"}`;
}

function isChatProject(candidate: unknown): candidate is ChatProject {
  if (typeof candidate !== "object" || candidate === null) {
    return false;
  }
  const project = candidate as Record<string, unknown>;
  return typeof project.id === "string" && typeof project.name === "string";
}

function normalizeProject(project: ChatProject): ChatProject {
  return {
    id: project.id,
    name: project.name.trim(),
    path: project.path?.trim() || null,
    templateId: project.templateId?.trim() || null,
    updatedAt: Number.isFinite(project.updatedAt) ? project.updatedAt : 0,
    chatCount: Number.isFinite(project.chatCount) ? project.chatCount : 0,
  };
}

export function mergeChatProjects(
  storedProjects: ChatProject[],
  metadataProjects: ChatProject[],
) {
  const projects = new Map<string, ChatProject>();
  for (const project of storedProjects) {
    const normalized = normalizeProject(project);
    if (normalized.name) {
      projects.set(normalized.id, normalized);
    }
  }
  for (const project of metadataProjects) {
    const normalized = normalizeProject(project);
    const existing = projects.get(normalized.id);
    projects.set(normalized.id, {
      id: normalized.id,
      name: existing?.name ?? normalized.name,
      path: existing ? existing.path : normalized.path,
      templateId: existing ? existing.templateId : normalized.templateId,
      updatedAt: Math.max(normalized.updatedAt, existing?.updatedAt ?? 0),
      chatCount: normalized.chatCount,
    });
  }
  return [...projects.values()].sort((left, right) => {
    if (left.updatedAt !== right.updatedAt) {
      return right.updatedAt - left.updatedAt;
    }
    return left.name.localeCompare(right.name);
  });
}

function readStoredChatProjects(workspaceId: string | null | undefined) {
  if (typeof window === "undefined") {
    return [];
  }
  try {
    const raw = window.localStorage.getItem(storageKey(workspaceId));
    if (!raw) {
      return [];
    }
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }
    return parsed.filter(isChatProject).map(normalizeProject);
  } catch {
    return [];
  }
}

function writeStoredChatProjects(
  workspaceId: string | null | undefined,
  projects: ChatProject[],
) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(
      storageKey(workspaceId),
      JSON.stringify(projects),
    );
    window.dispatchEvent(new CustomEvent(STORAGE_EVENT));
  } catch {
    // Local project cache is a convenience layer; ignore unavailable storage.
  }
}

export function upsertStoredChatProject(
  workspaceId: string | null | undefined,
  project: ChatProject,
) {
  const normalized = normalizeProject(project);
  if (!normalized.name) {
    return;
  }
  const projects = readStoredChatProjects(workspaceId);
  writeStoredChatProjects(workspaceId, [
    normalized,
    ...projects.filter((item) => item.id !== normalized.id),
  ]);
}

export function useStoredChatProjects(workspaceId: string | null | undefined) {
  const [projects, setProjects] = React.useState<ChatProject[]>(() =>
    readStoredChatProjects(workspaceId),
  );

  React.useEffect(() => {
    const refresh = () => setProjects(readStoredChatProjects(workspaceId));
    refresh();
    window.addEventListener(STORAGE_EVENT, refresh);
    window.addEventListener("storage", refresh);
    return () => {
      window.removeEventListener(STORAGE_EVENT, refresh);
      window.removeEventListener("storage", refresh);
    };
  }, [workspaceId]);

  return projects;
}
