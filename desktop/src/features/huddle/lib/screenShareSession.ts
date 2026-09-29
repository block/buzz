import {
  Room,
  RoomEvent,
  Track,
  type LocalTrackPublication,
  type RemoteTrack,
  type RoomOptions,
} from "livekit-client";
import { acquireDisplayMedia, stopMediaStreamTracks } from "./screenShareMedia";

export { acquireDisplayMedia, stopMediaStreamTracks } from "./screenShareMedia";

export type ScreenShareRemote = {
  participantIdentity: string;
  track: MediaStreamTrack;
  stream: MediaStream;
};

export type ScreenShareSessionCallbacks = {
  onRemoteChanged: (remote: ScreenShareRemote | null) => void;
  onLocalPreviewChanged: (stream: MediaStream | null) => void;
  onCurrentSharerChanged: (pubkey: string | null) => void;
  onDisconnected?: () => void;
};

/**
 * Thin LiveKit session for one huddle screen track.
 * Audio stays on the Opus WebSocket path — this only handles video.
 */

export class HuddleScreenShareSession {
  private room: Room | null = null;
  private localPublication: LocalTrackPublication | null = null;
  private localStream: MediaStream | null = null;
  private remote: ScreenShareRemote | null = null;
  private disposed = false;

  constructor(private readonly callbacks: ScreenShareSessionCallbacks) {}

  get isConnected(): boolean {
    return this.room?.state === "connected";
  }

  get isSharing(): boolean {
    return this.localPublication != null;
  }

  async connect(url: string, token: string): Promise<void> {
    if (this.disposed) return;
    if (this.room) {
      await this.disconnect();
    }
    // livekit-client ≥2.17 defaults singlePeerConnection=true (/rtc/v1).
    // Hula LiveKit was on v1.8.4 which only serves legacy /rtc — force dual-PC
    // so connect does not burn a failed v1 attempt before fallback.
    const options: RoomOptions = {
      adaptiveStream: true,
      dynacast: true,
      singlePeerConnection: false,
    };
    const room = new Room(options);
    this.room = room;

    room.on(RoomEvent.TrackSubscribed, (track, _pub, participant) => {
      if (track.kind !== Track.Kind.Video) return;
      this.attachRemote(participant.identity, track);
    });
    room.on(RoomEvent.TrackUnsubscribed, (track) => {
      if (track.kind !== Track.Kind.Video) return;
      if (this.remote?.track === track.mediaStreamTrack) {
        this.clearRemote();
      }
    });
    room.on(RoomEvent.ParticipantDisconnected, (participant) => {
      if (this.remote?.participantIdentity === participant.identity) {
        this.clearRemote();
      }
    });
    room.on(RoomEvent.Disconnected, () => {
      this.clearRemote();
      this.callbacks.onDisconnected?.();
    });

    await room.connect(url, token);

    // Attach any already-published remote screen tracks.
    for (const participant of room.remoteParticipants.values()) {
      for (const publication of participant.trackPublications.values()) {
        if (
          publication.kind === Track.Kind.Video &&
          publication.track &&
          publication.source === Track.Source.ScreenShare
        ) {
          this.attachRemote(participant.identity, publication.track);
        } else if (publication.kind === Track.Kind.Video && publication.track) {
          // Accept any remote video (room is screen-only by grant).
          this.attachRemote(participant.identity, publication.track);
        }
      }
    }
  }

  /**
   * Publish a pre-acquired display MediaStream.
   * Call {@link acquireDisplayMedia} first (inside the user-gesture handler)
   * so WebKit/WKWebView keeps the gesture chain intact.
   */
  async startShare(stream: MediaStream): Promise<void> {
    if (!this.room || this.disposed) {
      throw new Error("screen share room is not connected");
    }
    if (this.localPublication) {
      stopMediaStreamTracks(stream);
      return;
    }

    const [videoTrack] = stream.getVideoTracks();
    if (!videoTrack) {
      stopMediaStreamTracks(stream);
      throw new Error("no screen video track from getDisplayMedia");
    }
    videoTrack.addEventListener("ended", () => {
      void this.stopShare();
    });

    this.localStream = stream;
    this.callbacks.onLocalPreviewChanged(stream);
    this.localPublication = await this.room.localParticipant.publishTrack(
      videoTrack,
      {
        source: Track.Source.ScreenShare,
        name: "screen",
      },
    );
  }

  async stopShare(): Promise<void> {
    const publication = this.localPublication;
    this.localPublication = null;
    if (publication && this.room) {
      try {
        await this.room.localParticipant.unpublishTrack(publication.track!);
      } catch {
        /* best-effort */
      }
    }
    this.stopLocalTracks();
  }

  async disconnect(): Promise<void> {
    await this.stopShare();
    const room = this.room;
    this.room = null;
    this.clearRemote();
    if (room) {
      try {
        await room.disconnect(true);
      } catch {
        /* best-effort */
      }
    }
  }

  dispose(): void {
    this.disposed = true;
    void this.disconnect();
  }

  private attachRemote(
    identity: string,
    track: RemoteTrack | { mediaStreamTrack: MediaStreamTrack },
  ) {
    const media =
      "mediaStreamTrack" in track
        ? track.mediaStreamTrack
        : (track as RemoteTrack).mediaStreamTrack;
    const stream = new MediaStream([media]);
    this.remote = {
      participantIdentity: identity,
      track: media,
      stream,
    };
    this.callbacks.onRemoteChanged(this.remote);
    this.callbacks.onCurrentSharerChanged(identity);
  }

  private clearRemote() {
    if (!this.remote) return;
    this.remote = null;
    this.callbacks.onRemoteChanged(null);
    if (!this.localPublication) {
      this.callbacks.onCurrentSharerChanged(null);
    }
  }

  private stopLocalTracks() {
    if (!this.localStream) return;
    this.localStream.getTracks().forEach((t) => t.stop());
    this.localStream = null;
    this.callbacks.onLocalPreviewChanged(null);
  }
}
