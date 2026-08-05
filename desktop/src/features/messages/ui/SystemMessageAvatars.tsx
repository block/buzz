import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { getAvatarSizeRem } from "@/shared/lib/avatarScale";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useAvatarScale } from "@/shared/lib/useAvatarScale";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const MAX_MEMBERSHIP_AVATARS = 5;

function resolveAvatarUrl(
  pubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
): string | null {
  if (!pubkey || !profiles) return null;
  return profiles[pubkey.toLowerCase()]?.avatarUrl ?? null;
}

function isKnownAgentPubkey(
  pubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
  personaLookup?: Map<string, string>,
  agentPubkeys?: ReadonlySet<string>,
) {
  if (!pubkey) return false;
  const normalizedPubkey = normalizePubkey(pubkey);
  return (
    agentPubkeys?.has(normalizedPubkey) === true ||
    profiles?.[normalizedPubkey]?.isAgent === true ||
    personaLookup?.has(normalizedPubkey) === true
  );
}

export function SystemMessageAvatar({
  actorPubkey,
  agentPubkeys,
  currentPubkey,
  personaLookup,
  profiles,
  targetPubkey,
}: {
  actorPubkey: string | undefined;
  agentPubkeys?: ReadonlySet<string>;
  currentPubkey: string | undefined;
  personaLookup?: Map<string, string>;
  profiles: UserProfileLookup | undefined;
  targetPubkey: string | undefined;
}) {
  const avatarScale = useAvatarScale();
  // Dual-stack frame matches the md message avatar so rails stay aligned.
  const mdBoxRem = getAvatarSizeRem("md", avatarScale);
  const hasActorAndTarget =
    actorPubkey && targetPubkey && actorPubkey !== targetPubkey;
  const actorLabel = actorPubkey
    ? resolveUserLabel({
        pubkey: actorPubkey,
        currentPubkey,
        profiles,
        preferResolvedSelfLabel: true,
      })
    : "Someone";
  const singlePubkey = actorPubkey ?? targetPubkey;

  if (!hasActorAndTarget) {
    const isSingleAgent = isKnownAgentPubkey(
      singlePubkey,
      profiles,
      personaLookup,
      agentPubkeys,
    );
    const avatar = (
      <UserAvatar
        avatarUrl={resolveAvatarUrl(singlePubkey, profiles)}
        className="shrink-0 text-2xs"
        displayName={actorLabel}
        testId="system-message-avatar"
      />
    );
    if (singlePubkey) {
      return (
        <UserProfilePopover
          botIdenticonValue={isSingleAgent ? actorLabel : undefined}
          pubkey={singlePubkey}
          role={isSingleAgent ? "bot" : undefined}
        >
          <button
            className="shrink-0 rounded-full focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            data-testid="system-message-avatar"
            type="button"
          >
            {avatar}
          </button>
        </UserProfilePopover>
      );
    }
    return avatar;
  }

  const isActorAgent = isKnownAgentPubkey(
    actorPubkey,
    profiles,
    personaLookup,
    agentPubkeys,
  );
  const targetLabel = resolveUserLabel({
    pubkey: targetPubkey,
    currentPubkey,
    profiles,
    preferResolvedSelfLabel: true,
  });
  const dualAvatar = (
    <div
      className="relative shrink-0"
      data-testid="system-message-avatar"
      style={{
        width: `${mdBoxRem}rem`,
        height: `${mdBoxRem}rem`,
      }}
    >
      <UserAvatar
        avatarUrl={resolveAvatarUrl(actorPubkey, profiles)}
        className="border-2 border-background text-2xs"
        displayName={actorLabel}
        size="sm"
      />
      <UserAvatar
        avatarUrl={resolveAvatarUrl(targetPubkey, profiles)}
        className="!absolute !bottom-0 !right-0 border-2 border-background text-2xs"
        displayName={targetLabel}
        size="sm"
      />
    </div>
  );
  return (
    <UserProfilePopover
      botIdenticonValue={isActorAgent ? actorLabel : undefined}
      pubkey={actorPubkey}
      role={isActorAgent ? "bot" : undefined}
    >
      <button
        className="shrink-0 rounded-full focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        type="button"
      >
        {dualAvatar}
      </button>
    </UserProfilePopover>
  );
}

export function MembershipAvatarStack({
  currentPubkey,
  profiles,
  pubkeys,
}: {
  currentPubkey: string | undefined;
  profiles: UserProfileLookup | undefined;
  pubkeys: readonly string[];
}) {
  const avatarScale = useAvatarScale();
  const smRem = getAvatarSizeRem("sm", avatarScale);
  const maskRadiusRem = smRem * 0.58;
  const maskOffsetRem = smRem * 0.25;
  const overlapRem = smRem * 0.17;
  const visiblePubkeys = pubkeys.slice(0, MAX_MEMBERSHIP_AVATARS);
  if (visiblePubkeys.length === 0) return null;
  return (
    <div
      aria-label={`${visiblePubkeys.length} channel member${visiblePubkeys.length === 1 ? "" : "s"}`}
      className="relative z-10 flex shrink-0 items-center justify-center"
      data-testid="system-message-avatar-stack"
      role="img"
    >
      {visiblePubkeys.map((pubkey, index) => {
        const label = resolveUserLabel({
          pubkey,
          currentPubkey,
          profiles,
          preferResolvedSelfLabel: true,
        });
        return (
          <div
            className="relative"
            data-testid="system-message-avatar"
            key={pubkey}
            style={{
              zIndex: index + 1,
              marginLeft: index > 0 ? `-${overlapRem}rem` : undefined,
            }}
          >
            <span
              className="block"
              style={{
                ...(index < visiblePubkeys.length - 1 && {
                  mask: `radial-gradient(circle ${maskRadiusRem}rem at calc(100% + ${maskOffsetRem}rem) 50%, transparent 99%, #fff 100%)`,
                  WebkitMask: `radial-gradient(circle ${maskRadiusRem}rem at calc(100% + ${maskOffsetRem}rem) 50%, transparent 99%, #fff 100%)`,
                }),
              }}
            >
              <UserAvatar
                avatarUrl={resolveAvatarUrl(pubkey, profiles)}
                className="shrink-0 text-2xs"
                displayName={label}
                size="sm"
              />
            </span>
          </div>
        );
      })}
    </div>
  );
}
