import { useParticipants } from "@livekit/components-react";
import { Lock, MicOff, Shield, UserX } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  participantActionPayload,
  roomTogglePayload,
} from "@/features/meetings/moderationPayloads";
import type { ModerationAction } from "@/features/meetings/relay";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

export type ModerateInput = {
  action: ModerationAction;
  payload: Record<string, unknown>;
};

type CallControlBarProps = {
  roomName: string;
  /** Local participant identity, so it's excluded from the kick/mute list. */
  localIdentity: string | undefined;
  onModerate: (input: ModerateInput) => Promise<unknown>;
  isModeratePending: boolean;
};

/**
 * Host-only control menu. Sits alongside the prebuilt `<VideoConference />`
 * control bar (which already owns mic / camera / screen-share / leave for the
 * local participant). Rendered by `CallView` only when the LiveKit token
 * carries an `owner` / `moderator` claim — HiveTalk still enforces every action
 * server-side.
 */
export function CallControlBar({
  roomName,
  localIdentity,
  onModerate,
  isModeratePending,
}: CallControlBarProps) {
  // Optimistic room-level toggle state — HiveTalk doesn't surface the current
  // value over the SDK, so we track our own edits and start from "off".
  const [locked, setLocked] = React.useState(false);
  const [muteOnJoin, setMuteOnJoin] = React.useState(false);

  const participants = useParticipants();
  const others = participants.filter(
    (p) => p.identity !== localIdentity && !p.isLocal,
  );

  const run = React.useCallback(
    async (input: ModerateInput): Promise<boolean> => {
      try {
        await onModerate(input);
        return true;
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Host action failed.",
        );
        return false;
      }
    },
    [onModerate],
  );

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          aria-label="Host controls"
          data-testid="meeting-host-controls"
          disabled={isModeratePending}
          size="icon"
          variant="secondary"
        >
          <Shield className="h-4 w-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuLabel>Host controls</DropdownMenuLabel>
        <DropdownMenuCheckboxItem
          checked={locked}
          onCheckedChange={(next) => {
            setLocked(next);
            void run({
              action: "room/notify-lock",
              payload: roomTogglePayload(roomName, next),
            }).then((ok) => {
              if (!ok) setLocked(!next);
            });
          }}
        >
          <Lock className="mr-2 h-4 w-4" />
          Lock room
        </DropdownMenuCheckboxItem>
        <DropdownMenuCheckboxItem
          checked={muteOnJoin}
          onCheckedChange={(next) => {
            setMuteOnJoin(next);
            void run({
              action: "room/mute-on-join",
              payload: roomTogglePayload(roomName, next),
            }).then((ok) => {
              if (!ok) setMuteOnJoin(!next);
            });
          }}
        >
          <MicOff className="mr-2 h-4 w-4" />
          Mute on join
        </DropdownMenuCheckboxItem>

        <DropdownMenuSeparator />
        <DropdownMenuSub>
          <DropdownMenuSubTrigger disabled={others.length === 0}>
            <MicOff className="mr-2 h-4 w-4" />
            Mute participant
          </DropdownMenuSubTrigger>
          <DropdownMenuSubContent>
            {others.map((p) => (
              <DropdownMenuItem
                key={`mute-${p.identity}`}
                onSelect={() =>
                  void run({
                    action: "mute-user",
                    payload: participantActionPayload(roomName, p.identity),
                  })
                }
              >
                {p.name || p.identity}
              </DropdownMenuItem>
            ))}
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger disabled={others.length === 0}>
            <UserX className="mr-2 h-4 w-4" />
            Remove participant
          </DropdownMenuSubTrigger>
          <DropdownMenuSubContent>
            {others.map((p) => (
              <DropdownMenuItem
                className="text-destructive"
                key={`kick-${p.identity}`}
                onSelect={() =>
                  void run({
                    action: "kick-user",
                    payload: participantActionPayload(roomName, p.identity),
                  })
                }
              >
                {p.name || p.identity}
              </DropdownMenuItem>
            ))}
          </DropdownMenuSubContent>
        </DropdownMenuSub>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
