import type { ChannelType } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function resolveManagedAgentSendAudience({
  channelType,
  dmParticipantPubkeys,
  explicitMentionPubkeys,
  managedAgentPubkeys,
}: {
  channelType: ChannelType | null;
  dmParticipantPubkeys: Iterable<string>;
  explicitMentionPubkeys: Iterable<string>;
  managedAgentPubkeys: Iterable<string>;
}): string[] {
  const managed = new Set([...managedAgentPubkeys].map(normalizePubkey));
  const candidates = [
    ...explicitMentionPubkeys,
    ...(channelType === "dm" ? dmParticipantPubkeys : []),
  ];

  return [
    ...new Set(
      candidates
        .map(normalizePubkey)
        .filter((pubkey) => pubkey && managed.has(pubkey)),
    ),
  ];
}
