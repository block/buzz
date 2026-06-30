import type { ParsedAgentConversationLink } from "@/features/agents/agentConversationLink";
import type { AgentConversationMarker } from "@/features/agents/agentConversations";
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
  agentConversationMarkers?: readonly AgentConversationMarker[];
  channels: Channel[];
  imetaByUrl?: ImetaLookup;
  linkPreviewHrefs: ReadonlySet<string>;
  mentionPubkeysByName?: Record<string, string>;
  onOpenChannel: (channelId: string) => void;
  onOpenAgentConversationLink: (link: ParsedAgentConversationLink) => void;
  onOpenMessageLink: (link: ParsedMessageLink) => void;
};

export type MarkdownProps = {
  agentConversationMarkers?: readonly AgentConversationMarker[];
  channelNames?: string[];
  className?: string;
  content: string;
  customEmoji?: CustomEmoji[];
  imetaByUrl?: ImetaLookup;
  interactive?: boolean;
  agentMentionPubkeysByName?: Record<string, string>;
  mentionNames?: string[];
  mentionPubkeysByName?: Record<string, string>;
  searchQuery?: string;
  videoReviewContext?: VideoReviewContext;
};
