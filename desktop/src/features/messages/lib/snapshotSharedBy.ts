import type { TimelineMessage } from "@/features/messages/types";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";

/** Resolve attachment provenance from the raw event signer, never actor tags. */
export function resolveSnapshotSharedBy(
  message: Pick<TimelineMessage, "signerPubkey">,
  profiles?: UserProfileLookup,
): string | undefined {
  if (!message.signerPubkey) return undefined;

  return resolveUserLabel({
    profiles,
    pubkey: message.signerPubkey,
  });
}
