export type ReplyPlacementMode = "thread" | "top-level" | "follow-scope";

export type ManagedAgentReplyPlacement = {
  replyPlacement: ReplyPlacementMode;
  replyPlacementOverride: ReplyPlacementMode | null;
};

export type CreateManagedAgentReplyPlacement = {
  replyPlacement?: ReplyPlacementMode;
};

export type UpdateManagedAgentReplyPlacement = {
  replyPlacement?: ReplyPlacementMode | null;
};

export type AgentPersonaReplyPlacement = {
  replyPlacement: ReplyPlacementMode | null;
};

export type PersonaBehaviorReplyPlacement = {
  replyPlacement?: ReplyPlacementMode;
};

export type GlobalAgentReplyPlacement = {
  reply_placement: ReplyPlacementMode | null;
};

export type RawManagedAgentReplyPlacement = {
  reply_placement?: ReplyPlacementMode;
  reply_placement_override?: ReplyPlacementMode | null;
};

export function normalizeRawManagedAgentReplyPlacement(
  value: RawManagedAgentReplyPlacement,
): {
  replyPlacement: ReplyPlacementMode;
  replyPlacementOverride: ReplyPlacementMode | null;
} {
  return {
    replyPlacement: value.reply_placement ?? "thread",
    replyPlacementOverride: value.reply_placement_override ?? null,
  };
}

export function createManagedAgentReplyPlacement(
  value: ReplyPlacementMode | undefined,
): { replyPlacement?: ReplyPlacementMode } {
  return { replyPlacement: value };
}
