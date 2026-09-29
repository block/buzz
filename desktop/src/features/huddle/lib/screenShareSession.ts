import {
  ConnectionState,
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
  /** LiveKit remote video — UI must attach() this for adaptiveStream. */
  videoTrack: RemoteTrack;
  mediaStreamTrack: MediaStreamTrack;
  stream: MediaStream;
};

export type ScreenShareSessionCallbacks = {
  onRemoteChanged: (remote: ScreenShareRemote | null) => void;
  onLocalPreviewChanged: (stream: MediaStream | null) => void;
  onCurrentSharerChanged: (pubkey: string | null) => void;
  /** Subscriber-only permanent disconnect (no local capture held). */
  onDisconnected?: () => void;
  /**
   * Local capture is still live but LiveKit dropped. Hook should remint a
   * publish token and republish — do not tear down "You are sharing" UI.
   */
  onPublishInterrupted?: () => void;
  /** True while LiveKit reconnects or while we wait to republish. */
  onReconnectingChanged?: (reconnecting: boolean) => void;
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
  /** True while LiveKit is mid-reconnect — do not clear remote UI. */
  private reconnecting = false;
  /** Delay clearing remote UI after unsubscribe (publisher republish blip). */
  private remoteClearTimer: ReturnType<typeof setTimeout> | null = null;
  /** Serialize connect/reconnect so subscribe and publish never interleave. */
  private connectTail: Promise<void> = Promise.resolve();

  constructor(private readonly callbacks: ScreenShareSessionCallbacks) {}

  get isConnected(): boolean {
    return this.room?.state === "connected";
  }

  get isSharing(): boolean {
    return this.localStream != null;
  }

  /** Live local capture held for preview / republish (tracks may still be live). */
  get heldLocalStream(): MediaStream | null {
    return this.localStream;
  }

  async connect(url: string, token: string): Promise<void> {
    const run = this.connectTail.then(() => this.connectExclusive(url, token));
    // Keep the chain alive after failures so later connects still serialize.
    this.connectTail = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  private async connectExclusive(url: string, token: string): Promise<void> {
    if (this.disposed) return;
    if (this.room) {
      // Keep local capture across token/room swaps so Share UI stays up.
      await this.detachRoomKeepLocal();
    }
    // livekit-client ≥2.17 defaults singlePeerConnection=true (/rtc/v1).
    // Hula LiveKit was on v1.8.4 which only serves legacy /rtc — force dual-PC
    // so connect does not burn a failed v1 attempt before fallback.
    // Screen-only room: full-quality share. adaptiveStream without
    // RemoteTrack.attach() freezes after the first frame then goes black on
    // expand remount; even with attach(), dock↔expand unmount briefly marks
    // the track invisible and pauses. Keep adaptive/dynacast off here.
    const options: RoomOptions = {
      adaptiveStream: false,
      dynacast: false,
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
      // During LiveKit reconnect, tracks briefly unsubscribe then return —
      // clearing here causes a false drop/reconnect flicker in the UI.
      if (this.reconnecting) return;
      if (
        this.remote?.videoTrack === track ||
        this.remote?.mediaStreamTrack === track.mediaStreamTrack
      ) {
        // Publisher sticky-republish unsubscribes then resubscribes — keep the
        // last frame briefly instead of blanking the viewer mid-blip.
        this.scheduleSoftClearRemote();
      }
    });
    room.on(RoomEvent.ParticipantDisconnected, (participant) => {
      if (this.reconnecting) return;
      if (this.remote?.participantIdentity === participant.identity) {
        this.clearRemote();
      }
    });
    room.on(RoomEvent.Reconnecting, () => {
      this.setReconnecting(true);
    });
    room.on(RoomEvent.Reconnected, () => {
      this.setReconnecting(false);
      // Re-attach any remote screen tracks that survived the blip.
      this.attachExistingRemoteTracks(room);
    });
    room.on(RoomEvent.ConnectionStateChanged, (state) => {
      const stateName = String(state);
      if (
        state === ConnectionState.Reconnecting ||
        stateName === "signalReconnecting" ||
        stateName === "reconnecting"
      ) {
        this.setReconnecting(true);
        return;
      }
      if (state === ConnectionState.Connected || stateName === "connected") {
        this.setReconnecting(false);
      }
    });
    room.on(RoomEvent.Disconnected, () => {
      // Intentionally disconnected (or permanent failure) — not a blip.
      this.setReconnecting(false);
      this.localPublication = null;
      // Room is dead; drop the handle so disconnect()/republish can proceed.
      if (this.room === room) {
        this.room = null;
      }
      this.clearRemote();
      if (this.localStream) {
        // Keep capture + preview; ask the hook to remint + republish.
        this.callbacks.onPublishInterrupted?.();
      } else {
        this.callbacks.onDisconnected?.();
      }
    });

    await room.connect(url, token);
    this.setReconnecting(false);
    this.attachExistingRemoteTracks(room);
  }

  private attachExistingRemoteTracks(room: Room) {
    for (const participant of room.remoteParticipants.values()) {
      for (const publication of participant.trackPublications.values()) {
        if (publication.kind !== Track.Kind.Video || !publication.track) {
          continue;
        }
        // RemoteParticipant publications yield RemoteTrack.
        this.attachRemote(
          participant.identity,
          publication.track as RemoteTrack,
        );
      }
    }
  }

  /**
   * Publish a pre-acquired display MediaStream.
   * Call {@link acquireDisplayMedia} first (inside the user-gesture handler)
   * so WebKit/WKWebView keeps the gesture chain intact.
   *
   * Safe to call again after {@link detachRoomKeepLocal} / publish interrupt
   * with the same live stream — republishes without re-prompting the picker.
   */
  async startShare(stream: MediaStream): Promise<void> {
    if (!this.room || this.disposed) {
      throw new Error("screen share room is not connected");
    }
    if (this.localPublication) {
      // Already publishing — drop a duplicate acquire if any.
      if (stream !== this.localStream) {
        stopMediaStreamTracks(stream);
      }
      return;
    }

    const [videoTrack] = stream.getVideoTracks();
    if (!videoTrack || videoTrack.readyState === "ended") {
      stopMediaStreamTracks(stream);
      throw new Error("no screen video track from getDisplayMedia");
    }
    // OS picker "Stop sharing" ends the track — only then tear down UI.
    if (!this.localStream || this.localStream !== stream) {
      videoTrack.addEventListener("ended", () => {
        void this.stopShare();
      });
    }

    this.localStream = stream;
    this.callbacks.onLocalPreviewChanged(stream);
    this.localPublication = await this.room.localParticipant.publishTrack(
      videoTrack,
      {
        source: Track.Source.ScreenShare,
        name: "screen",
      },
    );
    this.setReconnecting(false);
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
    this.setReconnecting(false);
  }

  async disconnect(): Promise<void> {
    await this.stopShare();
    await this.detachRoomKeepLocal();
  }

  /**
   * Drop the LiveKit room without stopping local capture — used when reminting
   * a publish token or when the SFU dies mid-share. Preview stays up.
   */
  async detachRoomKeepLocal(): Promise<void> {
    const publication = this.localPublication;
    this.localPublication = null;
    if (publication && this.room) {
      try {
        await this.room.localParticipant.unpublishTrack(publication.track!);
      } catch {
        /* best-effort — room may already be dead */
      }
    }
    const room = this.room;
    this.room = null;
    this.setReconnecting(false);
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

  private setReconnecting(active: boolean) {
    if (this.reconnecting === active) return;
    this.reconnecting = active;
    this.callbacks.onReconnectingChanged?.(active);
  }

  private attachRemote(identity: string, track: RemoteTrack) {
    this.cancelSoftClearRemote();
    const media = track.mediaStreamTrack;
    const stream = new MediaStream([media]);
    this.remote = {
      participantIdentity: identity,
      videoTrack: track,
      mediaStreamTrack: media,
      stream,
    };
    this.callbacks.onRemoteChanged(this.remote);
    this.callbacks.onCurrentSharerChanged(identity);
  }

  private scheduleSoftClearRemote() {
    if (this.remoteClearTimer) return;
    this.remoteClearTimer = setTimeout(() => {
      this.remoteClearTimer = null;
      this.clearRemote();
    }, 2500);
  }

  private cancelSoftClearRemote() {
    if (!this.remoteClearTimer) return;
    clearTimeout(this.remoteClearTimer);
    this.remoteClearTimer = null;
  }

  private clearRemote() {
    this.cancelSoftClearRemote();
    if (!this.remote) return;
    this.remote = null;
    this.callbacks.onRemoteChanged(null);
    if (!this.localStream) {
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
