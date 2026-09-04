import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  useMeetingRoomsQuery,
  useMyMeetingRoomsQuery,
  useRegisterRoomMutation,
} from "@/features/meetings/hooks";
import { useMeetingsCapability } from "@/features/meetings/useMeetingsCapability";
import { MeetingsScreen } from "@/features/meetings/ui/MeetingsScreen";
import { SubscribeDialog } from "@/features/meetings/ui/SubscribeDialog";
import { SubscriptionStatusBadge } from "@/features/meetings/ui/SubscriptionStatusBadge";
import {
  isHostingSetupError,
  pendingInvoiceFromError,
  selectMeetingsView,
} from "@/features/meetings/ui/meetingsScreenState";

type MeetingsRouteScreenProps = {
  room?: string;
  action?: "join" | "start";
};

export function MeetingsRouteScreen({
  room,
  action,
}: MeetingsRouteScreenProps) {
  const { goMeetings } = useAppNavigation();
  const { capability, isLoading } = useMeetingsCapability();
  const roomsQuery = useMeetingRoomsQuery();
  const myRoomsQuery = useMyMeetingRoomsQuery();
  const registerRoom = useRegisterRoomMutation();

  const [subscribeOpen, setSubscribeOpen] = React.useState(false);
  /**
   * The room whose start is waiting on a subscription purchase, if any.
   * Single-use: set only by a start attempt that 402'd, cleared on every
   * terminal outcome and consumed by the retry, so a later unrelated purchase
   * (a renewal from the badge, "set up hosting") can never replay a stale room.
   */
  const pendingRoomRef = React.useRef<string | null>(null);

  const view = selectMeetingsView({
    hasCapability: capability !== null,
    isCapabilityLoading: isLoading,
    deepLink: { action, room },
  });

  const startMeeting = React.useCallback(
    (roomName: string) => {
      pendingRoomRef.current = roomName;
      registerRoom.mutate(roomName, {
        onError: (error) => {
          if (isHostingSetupError(error)) {
            setSubscribeOpen(true);
          } else {
            pendingRoomRef.current = null;
            toast.error(
              error instanceof Error
                ? error.message
                : "Couldn't start the meeting.",
            );
          }
        },
        onSuccess: (registered) => {
          pendingRoomRef.current = null;
          void goMeetings({ action: "join", room: registered.room_name });
        },
      });
    },
    [goMeetings, registerRoom],
  );

  return (
    <MeetingsScreen
      isRoomsLoading={roomsQuery.isLoading}
      myRooms={myRoomsQuery.data ?? []}
      onJoin={(target) => void goMeetings({ action: "join", room: target })}
      onLeaveCall={() => void goMeetings({ replace: true })}
      onRefresh={() => {
        void roomsQuery.refetch();
        void myRoomsQuery.refetch();
      }}
      onSetupHosting={() => {
        pendingRoomRef.current = null;
        setSubscribeOpen(true);
      }}
      onStart={startMeeting}
      registerError={registerRoom.error}
      registerPending={registerRoom.isPending}
      rooms={roomsQuery.data ?? []}
      subscribeDialog={
        <SubscribeDialog
          initialIntent={pendingInvoiceFromError(registerRoom.error)}
          onOpenChange={setSubscribeOpen}
          onSettled={() => {
            setSubscribeOpen(false);
            const target = pendingRoomRef.current;
            pendingRoomRef.current = null;
            if (target) startMeeting(target);
          }}
          open={subscribeOpen}
        />
      }
      subscriptionBadge={
        capability !== null ? (
          <SubscriptionStatusBadge
            onManage={() => {
              pendingRoomRef.current = null;
              setSubscribeOpen(true);
            }}
          />
        ) : null
      }
      view={view}
    />
  );
}
