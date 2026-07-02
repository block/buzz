import type { ChatMetadata } from "@/shared/api/types";

import type { ChatProject } from "@/features/chats/lib/chatSetup";

export const NO_PROJECT_ID = "__no_project__";

export function buildChatProjects(metadata: ChatMetadata[]): ChatProject[] {
  const projects = new Map<string, ChatProject>();

  for (const item of metadata) {
    const id = item.projectId?.trim();
    const name = item.projectName?.trim();
    if (!id || !name) {
      continue;
    }

    const existing = projects.get(id);
    if (existing) {
      existing.chatCount += 1;
      existing.updatedAt = Math.max(existing.updatedAt, item.updatedAt);
      if (!existing.path && item.projectPath?.trim()) {
        existing.path = item.projectPath.trim();
      }
      if (!existing.templateId && item.projectTemplateId?.trim()) {
        existing.templateId = item.projectTemplateId.trim();
      }
      continue;
    }

    projects.set(id, {
      id,
      name,
      path: item.projectPath?.trim() || null,
      templateId: item.projectTemplateId?.trim() || null,
      updatedAt: item.updatedAt,
      chatCount: 1,
    });
  }

  return [...projects.values()].sort((left, right) => {
    if (left.updatedAt !== right.updatedAt) {
      return right.updatedAt - left.updatedAt;
    }
    return left.name.localeCompare(right.name);
  });
}

export function chatProjectForMetadata(
  metadata: ChatMetadata | null | undefined,
): ChatProject | null {
  const id = metadata?.projectId?.trim();
  const name = metadata?.projectName?.trim();
  if (!id || !name) {
    return null;
  }
  return {
    id,
    name,
    path: metadata?.projectPath?.trim() || null,
    templateId: metadata?.projectTemplateId?.trim() || null,
    updatedAt: metadata?.updatedAt ?? 0,
    chatCount: 1,
  };
}
