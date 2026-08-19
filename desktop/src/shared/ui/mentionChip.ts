export const MENTION_CHIP_BASE_CLASSES = "mention-chip";

export const MENTION_CHIP_HOVER_CLASSES = "mention-chip-hover";

const INLINE_CHIP_LABEL_MAX_CHARACTERS = 48;

/** Caps a fragmentable chip label without changing its underlying metadata. */
export function truncateInlineChipLabel(label: string): string {
  const characters = Array.from(label);
  if (characters.length <= INLINE_CHIP_LABEL_MAX_CHARACTERS) return label;
  return `${characters
    .slice(0, INLINE_CHIP_LABEL_MAX_CHARACTERS - 1)
    .join("")}…`;
}

/** Allows a long chip to fragment into separately decorated line boxes. */
export const WRAPPING_INLINE_CHIP_CLASSES = "wrapping-inline-chip";

export type InlineChipIconKind =
  | "agent"
  | "human"
  | "channel"
  | "message"
  | "repo"
  | "project"
  | "pr"
  | "issue";

const INLINE_CHIP_ICON_KIND_CLASSES: Record<InlineChipIconKind, string> = {
  agent: "inline-chip-icon-agent agent-mention-highlight",
  human: "inline-chip-icon-human human-mention-highlight",
  channel: "inline-chip-icon-channel",
  message: "inline-chip-icon-message",
  repo: "inline-chip-icon-repo",
  project: "inline-chip-icon-project",
  pr: "inline-chip-icon-pr",
  issue: "inline-chip-icon-issue",
};

/** Shared icon-box contract for React chips and ProseMirror decorations. */
export function inlineChipIconClasses(kind: InlineChipIconKind): string {
  return `inline-chip-with-icon ${INLINE_CHIP_ICON_KIND_CLASSES[kind]}`;
}

/** Wrapper on rendered message Markdown — scopes inline chip CSS. */
export const MESSAGE_MARKDOWN_CLASS = "message-markdown";

/** Inline `` `code` `` chip — matches mention chip rhythm in message bodies. */
export const INLINE_CODE_CHIP_CLASS = "inline-code-chip";
