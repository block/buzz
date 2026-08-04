import { useChannelMembersQuery } from "@/features/channels/hooks";
import { useAuthorColorResolver } from "@/features/dev-mode/lib/authorColors";
import { memberCountLabel } from "@/features/dev-mode/lib/memberCount";
import type { Channel } from "@/shared/api/types";
import { truncatePubkey } from "@/shared/lib/pubkey";

/**
 * Above this, the top bar collapses to a member count — channels can have
 * 1000+ members, and a name list neither fits nor deserves a roster fetch.
 */
export const MAX_INLINE_MEMBERS = 4;

/** Top-bar member summary; click opens the palette's member browser. */
export function DevChannelMembers({
  channel,
  onShowMembers,
}: {
  channel: Channel;
  onShowMembers: () => void;
}) {
  const showNames =
    channel.memberCount > 0 && channel.memberCount <= MAX_INLINE_MEMBERS;
  const membersQuery = useChannelMembersQuery(channel.id, showNames);
  const resolveColor = useAuthorColorResolver();

  if (channel.memberCount === 0) return null;

  const members = showNames ? (membersQuery.data ?? []) : [];

  return (
    <button
      className="pointer-events-auto flex min-w-0 cursor-pointer items-baseline gap-1.5 truncate text-muted-foreground/70 hover:text-muted-foreground"
      data-testid="dev-mode-channel-members"
      onClick={onShowMembers}
      title="View members"
      type="button"
    >
      {showNames && members.length > 0 ? (
        members.map((member) => (
          <span
            key={member.pubkey}
            style={{ color: resolveColor(member.pubkey) }}
          >
            {member.displayName || truncatePubkey(member.pubkey)}
          </span>
        ))
      ) : (
        <span>{memberCountLabel(channel.memberCount)}</span>
      )}
    </button>
  );
}
