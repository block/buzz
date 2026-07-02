import type { ManagedAgent } from "@/shared/api/types";
import type { ChannelTemplate } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type ChatProject = {
  id: string;
  name: string;
  path: string | null;
  templateId: string | null;
  updatedAt: number;
  chatCount: number;
};

export type ChatProjectSetup = {
  project?: ChatProject | null;
  templateName?: string | null;
  agent?: ManagedAgent | null;
};

export const NO_PROJECT_SELECTION_ID = "__no_project__";

const MAX_QUICK_START_TITLE_CHARS = 72;

export function deriveChatTitle(content: string) {
  const normalized = content.trim().replace(/\s+/g, " ");
  if (!normalized) {
    return "New chat";
  }
  if (normalized.length <= MAX_QUICK_START_TITLE_CHARS) {
    return normalized;
  }
  return `${normalized.slice(0, MAX_QUICK_START_TITLE_CHARS - 3)}...`;
}

export function uniqueMentionPubkeys(
  identityPubkey: string | undefined,
  mentionPubkeys: string[],
  defaultAgentPubkey: string | null,
) {
  const normalized = new Set(
    mentionPubkeys.map(normalizePubkey).filter(Boolean),
  );
  if (defaultAgentPubkey) {
    normalized.add(normalizePubkey(defaultAgentPubkey));
  }
  if (identityPubkey) {
    normalized.delete(normalizePubkey(identityPubkey));
  }
  return [...normalized];
}

export function makeChatProjectId() {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `project-${Date.now().toString(36)}`;
}

export function buildProjectSetupContext(setup: ChatProjectSetup) {
  const project = setup.project;
  const hasProject = Boolean(project?.name.trim());
  const hasTemplate = Boolean(setup.templateName?.trim());
  if (!hasProject && !hasTemplate) {
    return null;
  }

  const lines = ["Project setup"];
  if (project) {
    lines.push(`Project: ${project.name}`);
    if (project.path?.trim()) {
      lines.push(`Folder: ${project.path.trim()}`);
    }
  } else {
    lines.push("Project: none");
  }

  if (setup.templateName?.trim()) {
    lines.push(`Template: ${setup.templateName.trim()}`);
  }
  if (setup.agent?.name.trim()) {
    lines.push(`Agent: ${setup.agent.name.trim()}`);
  }

  return lines.join("\n");
}

export function buildChatCanvasContent({
  channelName,
  leadingContent,
  template,
}: {
  channelName: string;
  leadingContent?: string | null;
  template?: ChannelTemplate | null;
}) {
  const parts: string[] = [];
  const leading = leadingContent?.trim();
  if (leading) {
    parts.push(leading);
  }
  if (template?.canvasTemplate?.trim()) {
    parts.push(
      template.canvasTemplate
        .replace(/\{channel\.name\}/g, channelName)
        .replace(/\{template\.name\}/g, template.name),
    );
  }
  return parts.length > 0 ? parts.join("\n\n") : null;
}
