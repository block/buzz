export type ChannelType = "stream" | "forum" | "dm";
export type ChannelVisibility = "open" | "private";
export type ChannelRole = "owner" | "admin" | "member" | "guest" | "bot";
export type AgentReplyMode = "thread" | "inline";

export type Channel = {
  id: string;
  name: string;
  channelType: ChannelType;
  visibility: ChannelVisibility;
  description: string;
  topic: string | null;
  purpose: string | null;
  memberCount: number;
  memberPubkeys: string[];
  lastMessageAt: string | null;
  archivedAt: string | null;
  participants: string[];
  participantPubkeys: string[];
  isMember: boolean;
  ttlSeconds: number | null;
  ttlDeadline: string | null;
  agentReplyMode: AgentReplyMode;
  dmRequireMention: boolean;
};

export type ChannelDetail = Channel & {
  createdBy: string;
  createdAt: string;
  updatedAt: string;
  topicSetBy: string | null;
  topicSetAt: string | null;
  purposeSetBy: string | null;
  purposeSetAt: string | null;
  topicRequired: boolean;
  maxMembers: number | null;
  nip29GroupId: string | null;
};

export type ChannelMember = {
  pubkey: string;
  role: ChannelRole;
  isAgent: boolean;
  joinedAt: string;
  displayName: string | null;
};

export type CreateChannelInput = {
  name: string;
  channelType: Exclude<ChannelType, "dm">;
  visibility: ChannelVisibility;
  description?: string;
  ttlSeconds?: number;
};

export type OpenDmInput = {
  pubkeys: string[];
};

export type UpdateChannelInput = {
  channelId: string;
  name?: string;
  description?: string;
  visibility?: ChannelVisibility;
  /** Omit to leave unchanged, `null` to clear (permanent), or a positive number of seconds to set. */
  ttlSeconds?: number | null;
  agentReplyMode?: AgentReplyMode;
  dmRequireMention?: boolean;
};

export type SetChannelTopicInput = {
  channelId: string;
  topic: string;
};

export type SetChannelPurposeInput = {
  channelId: string;
  purpose: string;
};

export type CanvasResponse = {
  content: string | null;
  updatedAt: number | null;
  author: string | null;
};

export type SetCanvasInput = {
  channelId: string;
  content: string;
};

export type SetCanvasResult = {
  ok: boolean;
  eventId: string;
};

export type AddChannelMembersInput = {
  channelId: string;
  pubkeys: string[];
  role?: Exclude<ChannelRole, "owner">;
};

export type AddChannelMembersResult = {
  added: string[];
  errors: Array<{
    pubkey: string;
    error: string;
  }>;
};
