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
