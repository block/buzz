import * as React from "react";
import {
  ConnectionState,
  type RemoteParticipant,
  type RemoteTrack,
  type RemoteTrackPublication,
  Room,
  RoomEvent,
  Track,
} from "livekit-client";

import { nip98PostHeader } from "@/shared/api/nip98";
import type { AgentMediaSession } from "./agentMediaSession";

/** Bound the token request so an unreachable gateway surfaces in seconds. */
const TOKEN_REQUEST_TIMEOUT_MS = 15_000;

export type AgentMediaRoomStatus =
  | "idle"
  | "authorizing"
  | "connecting"
  | "connected"
  | "error";

export type AgentMediaRoomState = {
  status: AgentMediaRoomStatus;
  /** Human-readable failure, set only when `status` is `"error"`. */
  error: string | null;
  /**
   * Attach the agent's video to an element. Returns a detach function.
   *
   * Exposed as a callback rather than a raw track so the caller never has to
   * remember to detach — LiveKit reuses the underlying element otherwise, and a
   * stale attachment keeps a decoded video surface alive after the panel closes.
   */
  attachVideo: ((element: HTMLVideoElement) => () => void) | null;
  /** Whether the local microphone is publishing. */
  micEnabled: boolean;
  setMicEnabled: (enabled: boolean) => void;
  /** True when the session grants this viewer outbound audio. */
  canPublishAudio: boolean;
};

async function fetchViewerToken(session: AgentMediaSession): Promise<string> {
  if (!session.tokenEndpoint) {
    throw new Error("This session did not publish a token endpoint.");
  }
  const body = JSON.stringify({
    channel_id: session.channelId,
    session_event_id: session.eventId,
    room: session.room,
  });
  // The gateway learns *who* is asking from the NIP-98 event's signature, then
  // checks that pubkey's membership of the announcing channel before minting.
  // The client never holds a provider API secret.
  const authorization = await nip98PostHeader(session.tokenEndpoint, body);
  const response = await fetch(session.tokenEndpoint, {
    method: "POST",
    headers: {
      Authorization: authorization,
      "Content-Type": "application/json",
    },
    body,
    signal: AbortSignal.timeout(TOKEN_REQUEST_TIMEOUT_MS),
  });
  const json = (await response.json().catch(() => ({}))) as Record<
    string,
    unknown
  >;
  if (!response.ok) {
    const message =
      typeof json.error === "string" ? json.error : `HTTP ${response.status}`;
    throw new Error(message);
  }
  if (typeof json.token !== "string" || json.token.length === 0) {
    throw new Error("Gateway returned no token.");
  }
  return json.token;
}

/**
 * Join the provider room for `session` and expose the agent's video.
 *
 * Subscribe-only by default: the microphone starts off and is never enabled
 * without an explicit call, because this hook mounts as soon as the panel opens
 * and silently hot-miking someone is not acceptable. Disconnects on unmount and
 * on session change — leaving the room open would keep a WebRTC connection (and
 * the provider's meter) running behind a closed panel.
 */
export function useAgentMediaRoom(
  session: AgentMediaSession | null,
): AgentMediaRoomState {
  const [status, setStatus] = React.useState<AgentMediaRoomStatus>("idle");
  const [error, setError] = React.useState<string | null>(null);
  const [videoTrack, setVideoTrack] = React.useState<RemoteTrack | null>(null);
  const [micEnabled, setMicEnabledState] = React.useState(false);
  const roomRef = React.useRef<Room | null>(null);

  const canPublishAudio = session?.viewer.publish.includes("audio") ?? false;

  React.useEffect(() => {
    if (!session) {
      setStatus("idle");
      setError(null);
      setVideoTrack(null);
      return;
    }

    let disposed = false;
    const room = new Room();
    roomRef.current = room;
    setMicEnabledState(false);

    const onSubscribed = (
      track: RemoteTrack,
      _publication: RemoteTrackPublication,
      participant: RemoteParticipant,
    ) => {
      if (disposed || track.kind !== Track.Kind.Video) return;
      // Only the announcing agent's video belongs in this panel — a room may
      // carry other publishers once sessions go multi-party, and rendering the
      // wrong face is worse than rendering none.
      //
      // The gateway is expected to put the agent's hex pubkey in its LiveKit
      // identity. When it does, match on that. When it does not, fall back to
      // the announcement: if it declares exactly one avatar publisher then a
      // single video track is unambiguous, and refusing it would strand v1 on
      // an identity convention the wire format never actually promised.
      const identity = participant.identity.toLowerCase();
      const identifiesAgent = identity.includes(session.agentPubkey);
      const soleAvatarPublisher =
        session.participants.filter((entry) =>
          entry.tracks.includes("avatar_video"),
        ).length === 1;
      if (!identifiesAgent && !soleAvatarPublisher) return;
      setVideoTrack(track);
    };

    const onUnsubscribed = (track: RemoteTrack) => {
      if (disposed) return;
      setVideoTrack((current) => (current === track ? null : current));
    };

    const onStateChanged = (state: ConnectionState) => {
      if (disposed) return;
      if (state === ConnectionState.Connected) setStatus("connected");
      else if (state === ConnectionState.Reconnecting) setStatus("connecting");
    };

    const onDisconnected = () => {
      if (disposed) return;
      setVideoTrack(null);
      setStatus("idle");
    };

    room
      .on(RoomEvent.TrackSubscribed, onSubscribed)
      .on(RoomEvent.TrackUnsubscribed, onUnsubscribed)
      .on(RoomEvent.ConnectionStateChanged, onStateChanged)
      .on(RoomEvent.Disconnected, onDisconnected);

    setStatus("authorizing");
    setError(null);

    void (async () => {
      try {
        const token = await fetchViewerToken(session);
        if (disposed) return;
        setStatus("connecting");
        await room.connect(session.serverUrl, token);
        if (disposed) {
          await room.disconnect();
          return;
        }
        setStatus("connected");
      } catch (cause) {
        if (disposed) return;
        setStatus("error");
        setError(cause instanceof Error ? cause.message : "Failed to connect.");
      }
    })();

    return () => {
      disposed = true;
      room
        .off(RoomEvent.TrackSubscribed, onSubscribed)
        .off(RoomEvent.TrackUnsubscribed, onUnsubscribed)
        .off(RoomEvent.ConnectionStateChanged, onStateChanged)
        .off(RoomEvent.Disconnected, onDisconnected);
      void room.disconnect();
      if (roomRef.current === room) roomRef.current = null;
      setVideoTrack(null);
    };
  }, [session]);

  const attachVideo = React.useMemo(() => {
    if (!videoTrack) return null;
    return (element: HTMLVideoElement) => {
      videoTrack.attach(element);
      return () => {
        videoTrack.detach(element);
      };
    };
  }, [videoTrack]);

  const setMicEnabled = React.useCallback((enabled: boolean) => {
    const room = roomRef.current;
    if (!room) return;
    setMicEnabledState(enabled);
    void room.localParticipant.setMicrophoneEnabled(enabled).catch(() => {
      setMicEnabledState(false);
    });
  }, []);

  return {
    status,
    error,
    attachVideo,
    micEnabled,
    setMicEnabled,
    canPublishAudio,
  };
}
