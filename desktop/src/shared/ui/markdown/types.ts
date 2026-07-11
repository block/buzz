import type { ParsedMessageLink } from "@/features/messages/lib/messageLink";
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
  href: string;
  interactive: boolean;
  link: ParsedMessageLink;
  onOpenMessageLink: (link: ParsedMessageLink) => void;
};

export type MarkdownRuntime = {
  agentMentionPubkeysByName?: Record<string, string>;
  channels: Channel[];
  imetaByUrl?: ImetaLookup;
  mentionPubkeysByName?: Record<string, string>;
  onOpenChannel: (channelId: string) => void;
  onOpenMessageLink: (link: ParsedMessageLink) => void;
  /**
   * Called by AgentSnapshotCard after a successful verified in-memory fetch.
   * The implementation should navigate to /agents and trigger the existing
   * snapshot import flow with the supplied bytes. Optional — when absent the
   * Import button is present but falls back to a no-op (the card is still
   * rendered on read-only surfaces such as the forum post renderer).
   */
  onImportSnapshotFromUrl?: (fileBytes: number[], fileName: string) => void;
};

export type MarkdownProps = {
  channelNames?: string[];
  className?: string;
  content: string;
  customEmoji?: CustomEmoji[];
  imetaByUrl?: ImetaLookup;
  interactive?: boolean;
  agentMentionPubkeysByName?: Record<string, string>;
  mentionNames?: string[];
  mentionPubkeysByName?: Record<string, string>;
  mediaInset?: boolean;
  searchQuery?: string;
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
