import type { RelayEvent } from "@/shared/api/types";

export function replyAgentRecipient(
  parentEventId: string,
  messageCaches: readonly (readonly Pick<
    RelayEvent,
    "id" | "pubkey" | "tags"
  >[])[],
  senderPubkey: string,
): string | null;
