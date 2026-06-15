import { X } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

type ParticipantListProps = {
  /** Pubkey hex strings from the Rust huddle state */
  participants: string[];
  activeSpeakers?: string[];
  /** Pubkeys of agent participants — rendered with a bot badge */
  agentPubkeys?: string[];
  /** Called when the user clicks the remove button on an agent avatar */
  onRemoveAgent?: (pubkey: string) => void;
  className?: string;
};

export function ParticipantList({
  participants,
  activeSpeakers,
  agentPubkeys,
  onRemoveAgent,
  className,
}: ParticipantListProps) {
  const { data } = useUsersBatchQuery(participants);
  const profiles = data?.profiles ?? {};
  const agentSet = React.useMemo(
    () => new Set(agentPubkeys ?? []),
    [agentPubkeys],
  );

  if (participants.length === 0) return null;

  return (
    <ul className={cn("flex list-none items-center gap-1", className)}>
      {participants.map((pubkey) => {
        const profile = profiles[pubkey.toLowerCase()];
        const hasProfile = profile?.displayName || profile?.avatarUrl;
        const isActive = activeSpeakers?.includes(pubkey);
        const isAgent = agentSet.has(pubkey);
        const ariaLabel = `${profile?.displayName || `Participant ${pubkey.slice(0, 8)}`}${isAgent ? " (agent)" : ""}`;

        return (
          <li key={pubkey} className="group/participant relative">
            {hasProfile ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <div aria-label={ariaLabel} role="img">
                    <ProfileAvatar
                      avatarUrl={profile.avatarUrl ?? null}
                      label={profile.displayName || pubkey.slice(0, 6)}
                      className={cn(
                        "h-7 w-7 rounded-lg text-2xs",
                        isActive &&
                          "ring-2 ring-green-500 ring-offset-1 ring-offset-background",
                      )}
                    />
                  </div>
                </TooltipTrigger>
                <TooltipContent>{profile.displayName || pubkey}</TooltipContent>
              </Tooltip>
            ) : (
              <HexAvatar
                pubkey={pubkey}
                activeSpeakers={activeSpeakers}
                ariaLabel={ariaLabel}
              />
            )}
            {isAgent &&
              (onRemoveAgent ? (
                <button
                  aria-label={`Remove ${profile?.displayName || "agent"} from huddle`}
                  className="absolute -right-1 -top-1 flex h-3.5 w-3.5 items-center justify-center rounded-full bg-destructive text-destructive-foreground opacity-0 transition-opacity group-hover/participant:opacity-100"
                  onClick={() => onRemoveAgent(pubkey)}
                  type="button"
                >
                  <X className="h-2.5 w-2.5" />
                </button>
              ) : (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span
                      aria-hidden="true"
                      className="absolute -bottom-1 -right-1 text-2xs leading-none"
                    >
                      🤖
                    </span>
                  </TooltipTrigger>
                  <TooltipContent>Agent</TooltipContent>
                </Tooltip>
              ))}
          </li>
        );
      })}
    </ul>
  );
}

/** Compact hex-prefix avatar for participants without a loaded profile. */
function HexAvatar({
  pubkey,
  activeSpeakers,
  ariaLabel,
}: {
  pubkey: string;
  activeSpeakers?: string[];
  ariaLabel?: string;
}) {
  const shortId = pubkey.slice(0, 6).toUpperCase();
  const parsed = parseInt(pubkey.slice(0, 4), 16);
  const hue = Number.isNaN(parsed) ? 0 : parsed % 360;
  const sat = Number.isNaN(parsed) ? 0 : 60;
  const isActive = activeSpeakers?.includes(pubkey);

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div
          aria-label={ariaLabel ?? `Participant ${pubkey.slice(0, 8)}`}
          role="img"
          className={cn(
            "flex h-7 w-7 items-center justify-center rounded-lg text-2xs font-semibold shadow-xs",
            isActive &&
              "ring-2 ring-green-500 ring-offset-1 ring-offset-background",
          )}
          style={{
            backgroundColor: `hsl(${hue}, ${sat}%, 55%)`,
            color: "#fff",
          }}
        >
          {shortId}
        </div>
      </TooltipTrigger>
      <TooltipContent>{pubkey}</TooltipContent>
    </Tooltip>
  );
}
