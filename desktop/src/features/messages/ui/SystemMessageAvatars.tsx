import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
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
        accent={isSingleAgent}
        avatarUrl={resolveAvatarUrl(singlePubkey, profiles)}
        className="!h-9 !w-9 shrink-0 text-2xs"
        displayName={actorLabel}
        shape={isSingleAgent ? "squircle" : "circle"}
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
            className={cn(
              "shrink-0 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
              isSingleAgent ? "rounded-[30%]" : "rounded-full",
            )}
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
  const isTargetAgent = isKnownAgentPubkey(
    targetPubkey,
    profiles,
    personaLookup,
    agentPubkeys,
  );
  const dualAvatar = (
    <div
      className="relative h-9 w-9 shrink-0"
      data-testid="system-message-avatar"
    >
      <UserAvatar
        accent={isActorAgent}
        avatarUrl={resolveAvatarUrl(actorPubkey, profiles)}
        className="!h-7 !w-7 border-2 border-background text-2xs"
        displayName={actorLabel}
        shape={isActorAgent ? "squircle" : "circle"}
      />
      <UserAvatar
        accent={isTargetAgent}
        avatarUrl={resolveAvatarUrl(targetPubkey, profiles)}
        className="!absolute !bottom-0 !right-0 !h-7 !w-7 border-2 border-background text-2xs"
        displayName={targetLabel}
        shape={isTargetAgent ? "squircle" : "circle"}
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
        className={cn(
          "shrink-0 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
          isActorAgent ? "rounded-[30%]" : "rounded-full",
        )}
        type="button"
      >
        {dualAvatar}
      </button>
    </UserProfilePopover>
  );
}

export function MembershipAvatarStack({
  agentPubkeys,
  currentPubkey,
  personaLookup,
  profiles,
  pubkeys,
}: {
  agentPubkeys?: ReadonlySet<string>;
  currentPubkey: string | undefined;
  personaLookup?: Map<string, string>;
  profiles: UserProfileLookup | undefined;
  pubkeys: readonly string[];
}) {
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
        const isAgent = isKnownAgentPubkey(
          pubkey,
          profiles,
          personaLookup,
          agentPubkeys,
        );
        const label = resolveUserLabel({
          pubkey,
          currentPubkey,
          profiles,
          preferResolvedSelfLabel: true,
        });
        return (
          <div
            className={cn("relative", index > 0 && "-ml-1")}
            data-testid="system-message-avatar"
            key={pubkey}
            style={{ zIndex: index + 1 }}
          >
            <span className="block">
              <UserAvatar
                accent={isAgent}
                avatarUrl={resolveAvatarUrl(pubkey, profiles)}
                className={cn(
                  "h-6 w-6 text-2xs",
                  index < visiblePubkeys.length - 1 && "ring-2 ring-background",
                )}
                displayName={label}
                shape={isAgent ? "squircle" : "circle"}
                size="sm"
              />
            </span>
          </div>
        );
      })}
    </div>
  );
}
