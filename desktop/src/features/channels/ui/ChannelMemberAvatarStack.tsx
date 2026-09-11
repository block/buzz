import * as React from "react";

import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import type { ChannelMember } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const MAX_VISIBLE_AVATARS = 3;

export function ChannelMemberAvatarStack({
  currentPubkey,
  members,
}: {
  currentPubkey?: string;
  members: ChannelMember[];
}) {
  const visibleMembers = members.slice(0, MAX_VISIBLE_AVATARS);
  const visiblePubkeys = React.useMemo(
    () => members.slice(0, MAX_VISIBLE_AVATARS).map((member) => member.pubkey),
    [members],
  );
  const profilesQuery = useUsersBatchQuery(visiblePubkeys);
  const profiles = profilesQuery.data?.profiles;
  const overflowCount = members.length - visibleMembers.length;
  const stackItemCount = visibleMembers.length + (overflowCount > 0 ? 1 : 0);

  if (members.length === 0) {
    return null;
  }

  return (
    <div
      className="flex shrink-0 items-center pl-3"
      data-testid="channel-management-member-avatar-stack"
    >
      {visibleMembers.map((member, index) => {
        const normalizedPubkey = normalizePubkey(member.pubkey);
        const profile = profiles?.[normalizedPubkey];
        const label = resolveUserLabel({
          currentPubkey,
          fallbackName: member.displayName,
          profiles,
          pubkey: member.pubkey,
        });

        return (
          <span
            className={index > 0 ? "-ml-2" : ""}
            data-testid="channel-management-member-avatar"
            key={normalizedPubkey}
            style={{ zIndex: index + 1 }}
          >
            <UserAvatar
              avatarUrl={profile?.avatarUrl ?? null}
              className="!h-8 !w-8 border-2 border-background text-2xs"
              displayName={label}
              fallbackDelayMs={0}
              shape={profile?.isAgent ? "squircle" : "circle"}
            />
          </span>
        );
      })}
      {overflowCount > 0 ? (
        <span
          className="-ml-2 flex h-8 w-8 items-center justify-center rounded-full border-2 border-background bg-muted text-2xs font-semibold text-muted-foreground"
          data-testid="channel-management-member-avatar-overflow"
          style={{ zIndex: stackItemCount }}
          title={`${overflowCount} more members`}
        >
          +{overflowCount}
        </span>
      ) : null}
    </div>
  );
}
