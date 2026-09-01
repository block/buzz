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
import {
  type AgentMediaSession,
  trackBelongsToSessionAgent,
} from "./agentMediaSession";
import { createMicRequestQueue, type MicRequestQueue } from "./micRequestQueue";

/** Bound the token request so an unreachable gateway surfaces in seconds. */
const TOKEN_REQUEST_TIMEOUT_MS = 15_000;

/**
 * Bound the token response.
 *
 * The endpoint is named by the announcement, so the address this client signs
 * a NIP-98 request to and reads a reply from is chosen by the announcing
 * agent. The request timeout already caps how long a reply may take, but a
 * fast link delivers a great deal in fifteen seconds, and the reply is a small
 * JSON object — kilobytes, not megabytes. Reading it in bounded chunks costs
 * nothing and removes the question.
 */
const MAX_TOKEN_RESPONSE_BYTES = 64 * 1024;

/**
 * Read a response body as JSON, refusing one that runs past the cap.
 *
 * Streams rather than checking `Content-Length`, because a body sent with
 * chunked encoding declares no length and the header is the sender's claim in
 * any case. Returns `{}` for a body that is absent or not JSON: the caller
 * distinguishes an HTTP failure from a malformed success on its own, and both
 * of those want the status, not a parse error.
 */
async function readCappedJson(
  response: Response,
): Promise<Record<string, unknown>> {
  if (!response.body) return {};
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > MAX_TOKEN_RESPONSE_BYTES) {
        await reader.cancel();
        throw new Error("The gateway's response was too large.");
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const body = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    body.set(chunk, offset);
    offset += chunk.byteLength;
  }
  try {
    const parsed: unknown = JSON.parse(new TextDecoder().decode(body));
    return typeof parsed === "object" && parsed !== null
      ? (parsed as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}

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
  /**
   * Attach the agent's audio to an element. Returns a detach function.
   *
   * Separate from `attachVideo` because a media element plays only the tracks
   * in the stream it was given: attaching the video track alone renders a
   * silent face, which is what shipped before this existed.
   */
  attachAudio: ((element: HTMLMediaElement) => () => void) | null;
  /**
   * True when the browser is refusing to start audio without a gesture.
   *
   * Autoplay policy is per-document and sticky, so opening the panel by
   * clicking usually satisfies it — but the token fetch and the room connect
   * happen after that click, and a viewer who arrived some other way has no
   * gesture at all. When this is set, call `enableAudio` from a real click.
   */
  audioBlocked: boolean;
  /** Start audio playback after a user gesture. Safe to call at any time. */
  enableAudio: () => void;
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
  const json = await readCappedJson(response);
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
 *
 * The identity of `session` is part of the contract, not only its contents: the
 * effect below re-runs whenever the object changes, and re-running means
 * disconnect, another viewer token and a fresh join. Pass the object
 * `foldLiveSessions` produced — which is what `useAgentMediaSession` returns —
 * never one re-derived per render.
 */
export function useAgentMediaRoom(
  session: AgentMediaSession | null,
): AgentMediaRoomState {
  const [status, setStatus] = React.useState<AgentMediaRoomStatus>("idle");
  const [error, setError] = React.useState<string | null>(null);
  const [videoTrack, setVideoTrack] = React.useState<RemoteTrack | null>(null);
  const [audioTrack, setAudioTrack] = React.useState<RemoteTrack | null>(null);
  const [audioBlocked, setAudioBlocked] = React.useState(false);
  const [micEnabled, setMicEnabledState] = React.useState(false);
  const roomRef = React.useRef<Room | null>(null);
  // One queue for the hook's lifetime, reading the room at apply time rather
  // than capturing it, so a room swap does not need a second queue.
  const micQueueRef = React.useRef<MicRequestQueue | null>(null);
  if (!micQueueRef.current) {
    micQueueRef.current = createMicRequestQueue(async (enabled) => {
      const room = roomRef.current;
      if (!room) return;
      await room.localParticipant.setMicrophoneEnabled(enabled);
    });
  }

  const canPublishAudio = session?.viewer.publish.includes("audio") ?? false;

  React.useEffect(() => {
    if (!session) {
      setStatus("idle");
      setError(null);
      setVideoTrack(null);
      setAudioTrack(null);
      setAudioBlocked(false);
      return;
    }

    let disposed = false;
    const room = new Room();
    roomRef.current = room;
    // Supersede any request still in flight for the previous room, so its
    // outcome cannot write state that describes a connection now gone.
    micQueueRef.current?.supersede();
    setMicEnabledState(false);

    const onSubscribed = (
      track: RemoteTrack,
      _publication: RemoteTrackPublication,
      participant: RemoteParticipant,
    ) => {
      if (disposed) return;
      const isVideo = track.kind === Track.Kind.Video;
      const isAudio = track.kind === Track.Kind.Audio;
      if (!isVideo && !isAudio) return;

      // Only the announcing agent's tracks belong in this panel; the rule
      // itself lives with the other announcement logic so it can be tested
      // without a room.
      if (
        !trackBelongsToSessionAgent(
          session,
          participant.identity,
          isVideo ? "video" : "audio",
        )
      ) {
        return;
      }

      if (isVideo) {
        setVideoTrack(track);
        return;
      }
      setAudioTrack(track);
    };

    const onUnsubscribed = (track: RemoteTrack) => {
      if (disposed) return;
      setVideoTrack((current) => (current === track ? null : current));
      setAudioTrack((current) => (current === track ? null : current));
    };

    const onStateChanged = (state: ConnectionState) => {
      if (disposed) return;
      if (state === ConnectionState.Connected) setStatus("connected");
      else if (state === ConnectionState.Reconnecting) setStatus("connecting");
    };

    const onDisconnected = () => {
      if (disposed) return;
      setVideoTrack(null);
      setAudioTrack(null);
      setStatus("idle");
    };

    // The browser, not LiveKit, decides whether audio may start. Track its
    // answer rather than assuming success, so a blocked room can offer a
    // gesture instead of playing silently and looking broken.
    const onAudioPlaybackChanged = () => {
      if (disposed) return;
      setAudioBlocked(!room.canPlaybackAudio);
    };

    room
      .on(RoomEvent.TrackSubscribed, onSubscribed)
      .on(RoomEvent.TrackUnsubscribed, onUnsubscribed)
      .on(RoomEvent.ConnectionStateChanged, onStateChanged)
      .on(RoomEvent.AudioPlaybackStatusChanged, onAudioPlaybackChanged)
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
        .off(RoomEvent.AudioPlaybackStatusChanged, onAudioPlaybackChanged)
        .off(RoomEvent.Disconnected, onDisconnected);
      void room.disconnect();
      if (roomRef.current === room) roomRef.current = null;
      setVideoTrack(null);
      setAudioTrack(null);
      setAudioBlocked(false);
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

  const attachAudio = React.useMemo(() => {
    if (!audioTrack) return null;
    return (element: HTMLMediaElement) => {
      audioTrack.attach(element);
      return () => {
        audioTrack.detach(element);
      };
    };
  }, [audioTrack]);

  const enableAudio = React.useCallback(() => {
    const room = roomRef.current;
    if (!room) return;
    // Success fires AudioPlaybackStatusChanged, which clears the flag; a
    // rejection leaves it set so the affordance stays available.
    void room.startAudio().catch(() => {});
  }, []);

  /**
   * Turn this viewer's microphone on or off.
   *
   * The indicator moves at once, because the member asked and waiting on the
   * device to agree makes the button feel broken. Reverting it is left to the
   * queue, which reverts only on behalf of the request still speaking for the
   * member — see {@link createMicRequestQueue} for why a stale revert here
   * would be a microphone somebody believes is off.
   */
  const setMicEnabled = React.useCallback((enabled: boolean) => {
    if (!roomRef.current) return;
    setMicEnabledState(enabled);
    void micQueueRef.current?.request(enabled, () => setMicEnabledState(false));
  }, []);

  return {
    status,
    error,
    attachVideo,
    attachAudio,
    audioBlocked,
    enableAudio,
    micEnabled,
    setMicEnabled,
    canPublishAudio,
  };
}
