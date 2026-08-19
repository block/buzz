import type * as React from "react";

import type { ParsedMessageLink } from "@/features/messages/lib/messageLink";
import type { ParsedEntityLink } from "@/shared/lib/entityLink";
import type { Channel } from "@/shared/api/types";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import type { VideoReviewContext } from "../VideoPlayer";

export type ImetaEntry = {
  dim?: string;
  image?: string;
  thumb?: string;
  m?: string;
  size?: number;
  filename?: string;
  duration?: number;
  /** SHA-256 hex of the attachment bytes (from imeta `x` field). */
  x?: string;
};

export type ImetaLookup = Map<string, ImetaEntry>;

export type MessageLinkPillProps = {
  channels: Channel[];
  /** Original permalink text, preserved for the context menu's Copy action. */
  href?: string;
  interactive: boolean;
  link: ParsedMessageLink;
  onOpenMessageLink: (link: ParsedMessageLink) => void;
  threadExcerpt?: string | null;
  variant?: "default" | "sent-from-thread";
};

export type MarkdownRuntime = {
  agentMentionPubkeysByName?: Record<string, string>;
  channels: Channel[];
  imetaByUrl?: ImetaLookup;
  /** Inline content supplied to the first prose-capable Markdown block. */
  leadingInlineContent?: React.ReactNode;
  mentionPubkeysByName?: Record<string, string>;
  onOpenChannel: (channelId: string) => void;
  /** Navigate to a Buzz git entity (`buzz://pr|issue|repo` deep link). */
  onOpenEntityLink: (link: ParsedEntityLink) => void;
  onOpenMessageLink: (link: ParsedMessageLink) => void;
  /**
   * The resolved relay origin (e.g. `https://buzz.block.builderlab.xyz`),
   * or `null` when not yet resolved. Used by the anchor component to
   * validate that clone-URL rewrites point to the active relay only.
   */
  relayOrigin: string | null;
  /** Display name of the message author sharing an agent snapshot. */
  snapshotSharedBy?: string;
  /**
   * Called by AgentSnapshotCard after a successful verified in-memory fetch.
   * The implementation should navigate to /agents and trigger the existing
   * snapshot import flow with the supplied bytes. Optional — when absent the
   * Import button is present but falls back to a no-op (the card is still
   * rendered on read-only surfaces such as the forum post renderer).
   */
  onImportSnapshotFromUrl?: (
    fileBytes: number[],
    fileName: string,
    snapshotKind: "agent" | "team",
  ) => void;
};

export type MarkdownProps = {
  channelNames?: string[];
  className?: string;
  content: string;
  customEmoji?: CustomEmoji[];
  /**
   * When true (default), single newlines become `<br>` — chat Enter behavior.
   * Git commit bodies are hard-wrapped at ~72 columns; pass false so those
   * wraps reflow with the panel instead of staying a narrow column.
   */
  hardLineBreaks?: boolean;
  imetaByUrl?: ImetaLookup;
  interactive?: boolean;
  agentMentionPubkeysByName?: Record<string, string>;
  mentionNames?: string[];
  mentionPubkeysByName?: Record<string, string>;
  mediaInset?: boolean;
  /** Event/message identity used only for local preview-image visibility. */
  messageId?: string;
  linkPreviewsSuppressed?: boolean;
  linkPreviewTags?: readonly (readonly string[])[];
  /** Inline content prepended inside the first rendered prose paragraph. */
  leadingInlineContent?: React.ReactNode;
  onRemoveLinkPreviewsForEveryone?: () => Promise<void>;
  searchQuery?: string;
  /** Display name shown in shared-agent card metadata. */
  snapshotSharedBy?: string;
  videoReviewContext?: VideoReviewContext;
  /**
   * When set and the nudge payload's agent_pubkey matches, renders the
   * config-nudge sentinel as an Attachment card and strips the fence from
   * displayed prose. Must be undefined/null for every non-message Markdown
   * surface — keeps card rendering opt-in so untrusted content cannot forge
   * a nudge card.
   */
  configNudgeAuthorPubkey?: string | null;
};
