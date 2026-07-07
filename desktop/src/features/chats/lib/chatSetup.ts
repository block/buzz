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

const MAX_CONVERSATION_TITLE_CHARS = 48;
const MIN_CONVERSATION_TITLE_CHARS = 4;

// Conversational lead-ins that carry no subject: greetings, vocatives, and
// politeness framing. Stripped repeatedly until the sentence core remains.
const TITLE_LEAD_IN_PATTERNS: RegExp[] = [
  /^(?:hey|hi|hello|hiya|howdy|yo|ok(?:ay)?|so|um+|uh+|please)[\s,!.:-]+/i,
  /^(?:fizz|there|everyone|team)[\s,!.:-]+/i,
  /^(?:can|could|would|will) (?:you|we) (?:please )?/i,
  /^(?:i(?:'d| would) like (?:you |us )?to|i (?:want|need) (?:you |us )?to|i(?:'m| am) (?:trying|looking) to)\s+/i,
  /^(?:help me (?:to |with )?|let'?s |quick question[:,]?\s*|question[:,]\s*)/i,
  /^(?:i was wondering (?:if (?:you|we) (?:can|could) )?)/i,
];

/**
 * Derive a succinct conversation title from a chat's opening message: strip
 * markdown/mention noise and conversational lead-ins ("hey can you help
 * me…"), keep the first sentence, and cap on a word boundary. Falls back to
 * [`deriveChatTitle`] when nothing meaningful survives.
 */
export function deriveConversationTitle(content: string) {
  let text = content
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/`([^`]*)`/g, "$1")
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/https?:\/\/\S+/g, " ")
    .replace(/nostr:\S+/g, " ")
    .replace(/[*_~#>]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();

  // Only the opening sentence names the goal; drop follow-on sentences.
  const sentenceEnd = text.search(/[.?!](?:\s|$)/);
  if (sentenceEnd >= MIN_CONVERSATION_TITLE_CHARS) {
    text = text.slice(0, sentenceEnd);
  }

  for (let pass = 0; pass < 6; pass++) {
    const before = text;
    for (const pattern of TITLE_LEAD_IN_PATTERNS) {
      text = text.replace(pattern, "");
    }
    if (text === before) {
      break;
    }
  }

  text = text.trim().replace(/[\s,;:.!?-]+$/, "");
  if (text.length < MIN_CONVERSATION_TITLE_CHARS) {
    return deriveChatTitle(content);
  }

  if (text.length > MAX_CONVERSATION_TITLE_CHARS) {
    const cut = text.lastIndexOf(" ", MAX_CONVERSATION_TITLE_CHARS);
    text = text.slice(
      0,
      cut > MIN_CONVERSATION_TITLE_CHARS ? cut : MAX_CONVERSATION_TITLE_CHARS,
    );
    text = text.replace(/[\s,;:.!?-]+$/, "");
  }

  return text.charAt(0).toUpperCase() + text.slice(1);
}

const OWNER_BRANCH_PREFIXES = ["kennylopez", "kennethlopez", "kenneth"];
const WORK_BRANCH_PREFIX_PATTERN =
  /^(?:feat|feature|fix|bugfix|bug|chore|docs?|tests?|refactor|cleanup|wip|hotfix|release|task|work)[-_.]+/i;

export function deriveBranchTitle(branch: string | null | undefined) {
  let text = branch?.trim() ?? "";
  if (!text) {
    return null;
  }

  text = text
    .replace(/^refs\/heads\//i, "")
    .replace(/^remotes\//i, "")
    .replace(/^origin\//i, "");
  text = text.split("/").filter(Boolean).pop() ?? text;

  for (const prefix of OWNER_BRANCH_PREFIXES) {
    const pattern = new RegExp(`^${prefix}[-_.]+`, "i");
    text = text.replace(pattern, "");
  }

  let previous = "";
  while (previous !== text) {
    previous = text;
    text = text.replace(WORK_BRANCH_PREFIX_PATTERN, "");
  }

  text = text
    .replace(/^[A-Z]+-\d+[-_.]+/i, "")
    .replace(/[-_.]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();

  if (text.length < MIN_CONVERSATION_TITLE_CHARS) {
    return null;
  }

  return text.charAt(0).toUpperCase() + text.slice(1);
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
