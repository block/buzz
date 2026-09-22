import { canManageMessageForCurrentUser } from "@/features/messages/lib/canManageMessage";
import type { TimelineMessage } from "@/features/messages/types";
import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds";
import type { UserProfileLookup } from "@/features/profile/lib/identity";

export function canSendMessageToChannel(
  message: TimelineMessage,
  currentPubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
): boolean {
  return (
    message.kind === KIND_STREAM_MESSAGE &&
    !message.pending &&
    canManageMessageForCurrentUser(message, currentPubkey, profiles)
  );
}

export function assertCanSendMessageToChannel(
  message: TimelineMessage,
  currentPubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
): void {
  if (message.kind !== KIND_STREAM_MESSAGE) {
    throw new Error(
      "Only ordinary channel messages can be sent to the channel.",
    );
  }
  if (message.pending) {
    throw new Error("Wait for the message to finish sending first.");
  }
  if (!canManageMessageForCurrentUser(message, currentPubkey, profiles)) {
    throw new Error("You can only send your own or your agents' messages.");
  }
}
