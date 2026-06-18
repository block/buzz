import { UsersRound, X } from "lucide-react";
import * as React from "react";

import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

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

export function HuddleParticipantsControl({
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

  const participantLabel =
    participants.length === 1
      ? "1 participant"
      : `${participants.length} participants`;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          aria-label={`Show huddle participants (${participants.length})`}
          className={cn(
            "buzz-huddle-control-button relative h-12 w-12 shrink-0 rounded-md p-0",
            className,
          )}
          size="icon"
          type="button"
          variant="secondary"
        >
          <UsersRound className="h-4 w-4" />
          {participants.length > 1 && (
            <span className="absolute -right-1 -top-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold leading-none text-primary-foreground shadow-xs tabular-nums">
              {participants.length}
            </span>
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="buzz-huddle-drawer buzz-huddle-popover w-72 p-3 text-foreground"
        side="top"
        sideOffset={10}
      >
        <div className="mb-3 flex items-center justify-between gap-3">
          <h2 className="text-sm font-medium">Participants</h2>
          <span className="shrink-0 text-xs text-foreground/60">
            {participantLabel}
          </span>
        </div>
        <ul className="flex max-h-64 list-none flex-col gap-1 overflow-y-auto">
          {participants.map((pubkey) => {
            const profile = profiles[pubkey.toLowerCase()];
            const displayName =
              profile?.displayName || `Participant ${pubkey.slice(0, 8)}`;
            const isActive = activeSpeakers?.includes(pubkey);
            const isAgent = agentSet.has(pubkey);

            return (
              <li
                className="flex min-w-0 items-center gap-2 rounded-md px-2 py-1.5"
                key={pubkey}
              >
                {profile?.displayName || profile?.avatarUrl ? (
                  <ProfileAvatar
                    avatarUrl={profile.avatarUrl ?? null}
                    label={profile.displayName || pubkey.slice(0, 6)}
                    className={cn(
                      "h-8 w-8 rounded-full text-2xs",
                      isActive &&
                        "ring-2 ring-green-500 ring-offset-1 ring-offset-background",
                    )}
                  />
                ) : (
                  <HexAvatar pubkey={pubkey} isActive={isActive} size="lg" />
                )}

                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">
                    {displayName}
                  </div>
                  <div className="truncate text-xs text-foreground/60">
                    {isActive ? "Speaking" : isAgent ? "Agent" : "In huddle"}
                  </div>
                </div>

                {isAgent && onRemoveAgent && (
                  <Button
                    aria-label={`Remove ${displayName} from huddle`}
                    className="h-7 w-7 shrink-0 text-foreground/65 hover:bg-destructive/15 hover:text-destructive"
                    onClick={() => void onRemoveAgent(pubkey)}
                    size="icon"
                    type="button"
                    variant="ghost"
                  >
                    <X className="h-4 w-4" />
                  </Button>
                )}
              </li>
            );
          })}
        </ul>
      </PopoverContent>
    </Popover>
  );
}

/** Compact hex-prefix avatar for participants without a loaded profile. */
function HexAvatar({
  pubkey,
  isActive,
  size = "md",
}: {
  pubkey: string;
  isActive?: boolean;
  size?: "md" | "lg";
}) {
  const shortId = pubkey.slice(0, 6).toUpperCase();
  const parsed = parseInt(pubkey.slice(0, 4), 16);
  const hue = Number.isNaN(parsed) ? 0 : parsed % 360;
  const sat = Number.isNaN(parsed) ? 0 : 60;

  return (
    <div
      aria-label={`Participant ${pubkey.slice(0, 8)}`}
      role="img"
      className={cn(
        "flex items-center justify-center rounded-full font-semibold shadow-xs",
        size === "lg" ? "h-8 w-8 text-2xs" : "h-7 w-7 text-2xs",
        isActive &&
          "ring-2 ring-green-500 ring-offset-1 ring-offset-background",
      )}
      style={{
        backgroundColor: `hsl(${hue}, ${sat}%, 55%)`,
        color: "#fff",
      }}
    >
      {size === "lg" ? shortId : <UsersRound className="h-4 w-4" />}
    </div>
  );
}
