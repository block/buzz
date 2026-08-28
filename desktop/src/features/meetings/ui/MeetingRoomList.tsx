import { Radio, Users } from "lucide-react";

import type { ActiveRoom } from "@/features/meetings/api";
import { isRoomLive } from "@/features/meetings/ui/meetingsScreenState";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";

type MeetingRoomListProps = {
  rooms: ActiveRoom[];
  onJoin: (room: string) => void;
  emptyLabel: string;
};

export function MeetingRoomList({
  rooms,
  onJoin,
  emptyLabel,
}: MeetingRoomListProps) {
  if (rooms.length === 0) {
    return (
      <p className="rounded-xl border border-dashed border-border/70 px-4 py-6 text-center text-sm text-muted-foreground">
        {emptyLabel}
      </p>
    );
  }

  return (
    <ul className="space-y-2" data-testid="meeting-room-list">
      {rooms.map((room) => {
        const live = isRoomLive(room.numParticipants);
        return (
          <li
            className="flex items-center gap-3 rounded-xl border border-border/70 bg-card/40 px-4 py-3"
            key={room.name}
          >
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="truncate text-sm font-medium">
                  {room.name}
                </span>
                {live ? (
                  <Badge className="gap-1 px-1.5 text-2xs" variant="success">
                    <Radio className="h-3 w-3" />
                    Live
                  </Badge>
                ) : null}
              </div>
              {live ? (
                <span className="mt-0.5 flex items-center gap-1 text-2xs text-muted-foreground">
                  <Users className="h-3 w-3" />
                  {room.numParticipants}
                </span>
              ) : null}
            </div>
            <Button onClick={() => onJoin(room.name)} size="sm">
              Join
            </Button>
          </li>
        );
      })}
    </ul>
  );
}
