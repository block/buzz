import "@livekit/components-styles";

import {
  LiveKitRoom,
  RoomAudioRenderer,
  VideoConference,
  useConnectionState,
  useLocalParticipant,
} from "@livekit/components-react";
import { ConnectionState, DisconnectReason } from "livekit-client";
import { ArrowLeft } from "lucide-react";
import * as React from "react";

import {
  useMeetingTokenQuery,
  useModerateRoomMutation,
} from "@/features/meetings/hooks";
import { CallConnectionBanner } from "@/features/meetings/ui/CallConnectionBanner";
import {
  CallControlBar,
  type ModerateInput,
} from "@/features/meetings/ui/CallControlBar";
import {
  type CallBannerModel,
  type CallConnectionPhase,
  type CallDisconnectReason,
  connectionBannerModel,
  disconnectBannerModel,
  formatCallElapsed,
  sessionCapBannerModel,
} from "@/features/meetings/ui/callSessionState";
import { isHostingSetupError } from "@/features/meetings/ui/meetingsScreenState";
import { decodeMeetingTokenClaims } from "@/features/meetings/api";
import { Button } from "@/shared/ui/button";

type CallViewProps = {
  room: string;
  onLeave: () => void;
  onSetupHosting: () => void;
};

/** Collapse `livekit-client`'s `DisconnectReason` into the buckets the pure
 * banner helper understands. */
function bucketDisconnect(
  reason: DisconnectReason | undefined,
): CallDisconnectReason {
  switch (reason) {
    case DisconnectReason.CLIENT_INITIATED:
      return "user";
    case DisconnectReason.DUPLICATE_IDENTITY:
      return "duplicate";
    case DisconnectReason.SERVER_SHUTDOWN:
    case DisconnectReason.PARTICIPANT_REMOVED:
    case DisconnectReason.ROOM_DELETED:
    case DisconnectReason.ROOM_CLOSED:
      return "server";
    default:
      return "unknown";
  }
}

/** Collapse `ConnectionState` into the pure helper's phase union. */
function phaseOf(state: ConnectionState): CallConnectionPhase {
  switch (state) {
    case ConnectionState.Connecting:
      return "connecting";
    case ConnectionState.Connected:
      return "connected";
    case ConnectionState.Reconnecting:
    case ConnectionState.SignalReconnecting:
      return "reconnecting";
    default:
      return "disconnected";
  }
}

/**
 * Real LiveKit call view — replaces the Phase 3 `CallViewPlaceholder`. Fetches a
 * room token through the relay proxy, mounts `<LiveKitRoom>` + the prebuilt
 * `<VideoConference />` grid (local mic / camera / screen-share / leave), layers
 * host controls when the token grants them, and surfaces connection + session-
 * cap state over the grid.
 */
export function CallView({ room, onLeave, onSetupHosting }: CallViewProps) {
  const tokenQuery = useMeetingTokenQuery(room);

  const connectingBanner = connectionBannerModel("connecting");
  if (tokenQuery.isLoading) {
    return (
      <CallShell>
        {connectingBanner ? (
          <CallConnectionBanner model={connectingBanner} variant="overlay" />
        ) : null}
        <BackButton onLeave={onLeave} />
      </CallShell>
    );
  }

  if (tokenQuery.isError || !tokenQuery.data) {
    const error = tokenQuery.error;
    const hosting = isHostingSetupError(error);
    const message =
      error instanceof Error ? error.message : "Couldn't join this meeting.";
    return (
      <div
        className="flex min-h-0 min-w-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center"
        data-testid="meeting-call-error"
      >
        <div className="space-y-1">
          <p className="text-base font-medium">
            {hosting ? "Hosting isn't set up yet" : "Can't join this meeting"}
          </p>
          <p className="max-w-sm text-sm text-muted-foreground">{message}</p>
        </div>
        <div className="flex gap-2">
          <Button onClick={onLeave} size="sm" variant="outline">
            <ArrowLeft className="h-4 w-4" />
            Back to meetings
          </Button>
          {hosting ? (
            <Button onClick={onSetupHosting} size="sm">
              Set up hosting
            </Button>
          ) : (
            <Button
              onClick={() => void tokenQuery.refetch()}
              size="sm"
              variant="secondary"
            >
              Try again
            </Button>
          )}
        </div>
      </div>
    );
  }

  const { token, url } = tokenQuery.data;
  return (
    <CallRoom
      onLeave={onLeave}
      room={room}
      token={token}
      url={url}
      onReconnectToken={() => void tokenQuery.refetch()}
    />
  );
}

type CallRoomProps = {
  room: string;
  token: string;
  url: string;
  onLeave: () => void;
  onReconnectToken: () => void;
};

function CallRoom({
  room,
  token,
  url,
  onLeave,
  onReconnectToken,
}: CallRoomProps) {
  const [disconnect, setDisconnect] =
    React.useState<CallDisconnectReason | null>(null);
  // Bumped on Rejoin. Keys `<LiveKitRoom>` so a rejoin tears down and rebuilds
  // the Room — refetching a still-valid token alone never actually reconnects,
  // and a fresh mount also resets the elapsed clock / session-cap warning.
  const [rejoinNonce, setRejoinNonce] = React.useState(0);
  const claims = React.useMemo(() => decodeMeetingTokenClaims(token), [token]);
  const canHost = claims.owner || claims.moderator;
  const moderate = useModerateRoomMutation(token);

  React.useEffect(() => {
    if (disconnect === "user") onLeave();
  }, [disconnect, onLeave]);

  return (
    <CallShell>
      <LiveKitRoom
        key={rejoinNonce}
        audio
        className="h-full w-full"
        connect
        data-lk-theme="default"
        onDisconnected={(reason) => setDisconnect(bucketDisconnect(reason))}
        serverUrl={url}
        token={token}
        video
      >
        <CallStage
          canHost={canHost}
          disconnect={disconnect}
          onLeave={onLeave}
          onModerate={(input) => moderate.mutateAsync(input)}
          isModeratePending={moderate.isPending}
          onRejoin={() => {
            setDisconnect(null);
            setRejoinNonce((n) => n + 1);
            onReconnectToken();
          }}
          room={room}
        />
        <RoomAudioRenderer />
      </LiveKitRoom>
    </CallShell>
  );
}

type CallStageProps = {
  room: string;
  canHost: boolean;
  disconnect: CallDisconnectReason | null;
  onLeave: () => void;
  onRejoin: () => void;
  onModerate: (input: ModerateInput) => Promise<unknown>;
  isModeratePending: boolean;
};

function CallStage(props: CallStageProps) {
  const { room, canHost, disconnect, onLeave, onRejoin } = props;
  const connectionState = useConnectionState();
  const phase = phaseOf(connectionState);
  const { localParticipant } = useLocalParticipant();

  const connectedAtRef = React.useRef<number | null>(null);
  const [elapsedMs, setElapsedMs] = React.useState(0);

  React.useEffect(() => {
    if (phase === "connected" && connectedAtRef.current === null) {
      connectedAtRef.current = Date.now();
    }
  }, [phase]);

  React.useEffect(() => {
    if (phase !== "connected") return;
    const tick = () => {
      if (connectedAtRef.current !== null) {
        setElapsedMs(Date.now() - connectedAtRef.current);
      }
    };
    tick();
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, [phase]);

  // A server-side disconnect is reported as-is ("The room closed"). We can't
  // tell a time-cap cut from a host ending the room — the LiveKit reason is the
  // same — so we no longer guess "reached its time limit" from elapsed time.
  // The soft session-cap banner still warns about the 4h ceiling while live.
  const terminalReason: CallDisconnectReason | null = disconnect;

  const terminalBanner: CallBannerModel = terminalReason
    ? disconnectBannerModel(terminalReason)
    : null;
  const liveBanner = terminalBanner ? null : connectionBannerModel(phase);
  const capBanner = terminalBanner ? null : sessionCapBannerModel(elapsedMs);

  return (
    <div className="relative flex h-full min-h-0 min-w-0 flex-1 flex-col">
      {terminalBanner ? (
        <CallConnectionBanner
          model={terminalBanner}
          onRejoin={onRejoin}
          variant="overlay"
        />
      ) : liveBanner && phase === "disconnected" ? (
        <CallConnectionBanner
          model={liveBanner}
          onRejoin={onRejoin}
          variant="overlay"
        />
      ) : liveBanner ? (
        <CallConnectionBanner model={liveBanner} variant="strip" />
      ) : capBanner ? (
        <CallConnectionBanner model={capBanner} variant="strip" />
      ) : null}

      <div className="pointer-events-none absolute right-3 top-3 z-30 flex items-center gap-2">
        {phase === "connected" ? (
          <span className="pointer-events-auto rounded-full bg-background/80 px-2 py-1 font-mono text-xs tabular-nums text-muted-foreground backdrop-blur-sm">
            {formatCallElapsed(elapsedMs)}
          </span>
        ) : null}
        {canHost ? (
          <div className="pointer-events-auto">
            <CallControlBar
              isModeratePending={props.isModeratePending}
              localIdentity={localParticipant?.identity}
              onModerate={props.onModerate}
              roomName={room}
            />
          </div>
        ) : null}
        <Button
          className="pointer-events-auto"
          data-testid="meeting-leave"
          onClick={onLeave}
          size="sm"
          variant="secondary"
        >
          <ArrowLeft className="h-4 w-4" />
          Leave
        </Button>
      </div>

      <div className="min-h-0 flex-1">
        <VideoConference />
      </div>
    </div>
  );
}

function CallShell({ children }: { children: React.ReactNode }) {
  return (
    <div
      className="relative flex min-h-0 min-w-0 flex-1 flex-col bg-background"
      data-testid="meeting-call-view"
    >
      {children}
    </div>
  );
}

function BackButton({ onLeave }: { onLeave: () => void }) {
  return (
    <div className="absolute left-3 top-3 z-30">
      <Button onClick={onLeave} size="sm" variant="secondary">
        <ArrowLeft className="h-4 w-4" />
        Back
      </Button>
    </div>
  );
}
