import {
  profileJobTitle,
  useMemberJobTitle,
} from "@/features/company-team/useMemberJobTitle";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";

/** First word of a display name, used in the manager attribution. */
export function firstName(name: string | null | undefined): string | null {
  const trimmed = name?.trim();
  return trimmed ? (trimmed.split(/\s+/)[0] ?? null) : null;
}

/**
 * The line under an agent's name in a direct message: its job title and who
 * manages it ("Business Development · managed by the owner"). Shows what is
 * known and nothing for what is not.
 */
export function DmAgentSubtitle({ pubkey }: { pubkey: string }) {
  const positionTitle = useMemberJobTitle(pubkey);
  const agentQuery = useUsersBatchQuery([pubkey]);
  const title =
    profileJobTitle(agentQuery.data?.profiles[normalizePubkey(pubkey)]) ??
    positionTitle;
  const ownerPubkey =
    agentQuery.data?.profiles[normalizePubkey(pubkey)]?.ownerPubkey ?? null;
  const ownerQuery = useUsersBatchQuery(ownerPubkey ? [ownerPubkey] : [], {
    enabled: Boolean(ownerPubkey),
  });
  const ownerName = ownerPubkey
    ? firstName(
        ownerQuery.data?.profiles[normalizePubkey(ownerPubkey)]?.displayName,
      )
    : null;
  const parts = [title, ownerName ? `managed by ${ownerName}` : null].filter(
    (part): part is string => Boolean(part),
  );
  return parts.length > 0 ? parts.join(" · ") : null;
}
