import { ArrowLeft, Video } from "lucide-react";

import { Button } from "@/shared/ui/button";

type CallViewPlaceholderProps = {
  room: string;
  onLeave: () => void;
};

/**
 * Phase 3 stand-in for the real LiveKit call view (Phase 4). It exists now so
 * the `/meetings?room=&action=join` deep-link and the join-button navigation
 * are wired and testable before the SDK lands. Phase 4 swaps this component for
 * one that calls `getMeetingToken` and mounts `<LiveKitRoom>`.
 */
export function CallViewPlaceholder({
  room,
  onLeave,
}: CallViewPlaceholderProps) {
  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center"
      data-testid="meeting-call-placeholder"
    >
      <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-muted text-muted-foreground">
        <Video className="h-7 w-7" />
      </div>
      <div className="space-y-1">
        <p className="text-base font-medium">Connecting to {room}…</p>
        <p className="text-sm text-muted-foreground">
          Live video arrives in Phase 4. The room is resolved and ready.
        </p>
      </div>
      <Button onClick={onLeave} size="sm" variant="outline">
        <ArrowLeft className="h-4 w-4" />
        Back to meetings
      </Button>
    </div>
  );
}
