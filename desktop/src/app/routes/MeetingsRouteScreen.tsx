import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  useMeetingRoomsQuery,
  useMyMeetingRoomsQuery,
  useRegisterRoomMutation,
} from "@/features/meetings/hooks";
import { useMeetingsCapability } from "@/features/meetings/useMeetingsCapability";
import { MeetingsScreen } from "@/features/meetings/ui/MeetingsScreen";
import {
  isHostingSetupError,
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

  const view = selectMeetingsView({
    hasCapability: capability !== null,
    isCapabilityLoading: isLoading,
    deepLink: { action, room },
  });

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
      onSetupHosting={() =>
        toast.info("Hosting setup arrives in a later update.")
      }
      onStart={(roomName) => {
        registerRoom.mutate(roomName, {
          onError: (error) => {
            if (!isHostingSetupError(error)) {
              toast.error(
                error instanceof Error
                  ? error.message
                  : "Couldn't start the meeting.",
              );
            }
          },
          onSuccess: (registered) =>
            void goMeetings({ action: "join", room: registered.room_name }),
        });
      }}
      registerError={registerRoom.error}
      registerPending={registerRoom.isPending}
      rooms={roomsQuery.data ?? []}
      view={view}
    />
  );
}
