import { useAuthorColorResolver } from "@/features/dev-mode/lib/authorColors";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import { truncatePubkey } from "@/shared/lib/pubkey";

const MAX_VISIBLE_MEMBERS = 5;

/** Colored member names for the top bar — who is in this chat with you. */
export function DevChannelMembers({ channelId }: { channelId: string }) {
  const membersQuery = useChannelMembersQuery(channelId);
  const resolveColor = useAuthorColorResolver();

  const members = membersQuery.data ?? [];
  if (members.length === 0) return null;

  const visible = members.slice(0, MAX_VISIBLE_MEMBERS);
  const overflow = members.length - visible.length;

  return (
    <span
      className="flex min-w-0 items-baseline gap-1.5 truncate text-muted-foreground/70"
      data-testid="dev-mode-channel-members"
    >
      ·
      {visible.map((member) => (
        <span
          key={member.pubkey}
          style={{ color: resolveColor(member.pubkey) }}
        >
          {member.displayName || truncatePubkey(member.pubkey)}
        </span>
      ))}
      {overflow > 0 ? <span>+{overflow}</span> : null}
    </span>
  );
}
