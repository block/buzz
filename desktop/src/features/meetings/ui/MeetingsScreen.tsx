import { RefreshCw } from "lucide-react";
import * as React from "react";

import { MeetingError } from "@/features/meetings/api";
import type { ActiveRoom } from "@/features/meetings/api";
import { LazyCallView } from "@/features/meetings/ui/lazyCallView";
import { MeetingRoomList } from "@/features/meetings/ui/MeetingRoomList";
import {
  isHostingSetupError,
  type MeetingsView,
} from "@/features/meetings/ui/meetingsScreenState";
import { StartMeetingForm } from "@/features/meetings/ui/StartMeetingForm";
import { Button } from "@/shared/ui/button";
import { PageHeader } from "@/shared/ui/PageHeader";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

type MeetingsScreenProps = {
  view: MeetingsView;
  rooms: ActiveRoom[];
  myRooms: ActiveRoom[];
  isRoomsLoading: boolean;
  registerPending: boolean;
  registerError: unknown;
  onJoin: (room: string) => void;
  onStart: (roomName: string) => void;
  onSetupHosting: () => void;
  onLeaveCall: () => void;
  onRefresh: () => void;
};

export function MeetingsScreen({
  view,
  rooms,
  myRooms,
  isRoomsLoading,
  registerPending,
  registerError,
  onJoin,
  onStart,
  onSetupHosting,
  onLeaveCall,
  onRefresh,
}: MeetingsScreenProps) {
  if (view.kind === "loading") {
    return <ViewLoadingFallback kind="meetings" />;
  }

  if (view.kind === "unavailable") {
    return (
      <div
        className="flex min-h-0 min-w-0 flex-1 flex-col items-center justify-center gap-2 p-8 text-center"
        data-testid="meetings-unavailable"
      >
        <p className="text-base font-medium">Meetings isn't available here</p>
        <p className="text-sm text-muted-foreground">
          This community's relay hasn't enabled video meetings.
        </p>
      </div>
    );
  }

  if (view.kind === "call") {
    return (
      <React.Suspense fallback={<ViewLoadingFallback kind="meetings" />}>
        <LazyCallView
          onLeave={onLeaveCall}
          onSetupHosting={onSetupHosting}
          room={view.room}
        />
      </React.Suspense>
    );
  }

  const hostingError = isHostingSetupError(registerError);
  const transientMessage =
    registerError instanceof MeetingError && !hostingError
      ? registerError.message
      : undefined;

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-6 px-4 py-6 sm:px-6">
        <PageHeader
          action={
            <Button
              aria-label="Refresh meetings"
              onClick={onRefresh}
              size="icon"
              variant="ghost"
            >
              <RefreshCw className="h-4 w-4" />
            </Button>
          }
          description="Live video rooms hosted on this community's relay."
          title="Meetings"
        />

        {myRooms.length > 0 ? (
          <section className="space-y-2" data-testid="my-meeting-rooms">
            <h2 className="text-sm font-medium text-muted-foreground">
              My rooms
            </h2>
            <MeetingRoomList emptyLabel="" onJoin={onJoin} rooms={myRooms} />
          </section>
        ) : null}

        <section className="space-y-2">
          <h2 className="text-sm font-medium text-muted-foreground">
            Active meetings
          </h2>
          {isRoomsLoading ? (
            <p className="rounded-xl border border-dashed border-border/70 px-4 py-6 text-center text-sm text-muted-foreground">
              Loading meetings…
            </p>
          ) : (
            <MeetingRoomList
              emptyLabel="No active meetings right now."
              onJoin={onJoin}
              rooms={rooms}
            />
          )}
        </section>

        <section className="space-y-2">
          <h2 className="text-sm font-medium text-muted-foreground">
            Start a meeting
          </h2>
          <StartMeetingForm
            autoFocus={view.focusStart}
            errorMessage={transientMessage}
            hostingError={hostingError}
            initialValue={view.prefillRoom ?? ""}
            isPending={registerPending}
            onSetupHosting={onSetupHosting}
            onSubmit={onStart}
          />
        </section>
      </div>
    </div>
  );
}
