import * as React from "react";

import type { RemoteTrack } from "livekit-client";

import {
  acquireDisplayMedia,
  HuddleScreenShareSession,
  stopMediaStreamTracks,
  type ScreenShareRemote,
} from "../lib/screenShareSession";
import { isShareBlockedByOther } from "../lib/screenSharePolicy";
import {
  mintScreenShareToken,
  stopScreenShareSlot,
  type ScreenTokenResponse,
} from "../lib/screenShareToken";

export type HuddleScreenShareState = {
  /** False when relay has no LiveKit config — hide Share. */
  available: boolean | null;
  connecting: boolean;
  sharing: boolean;
  /**
   * True while LiveKit is down mid-share and we are reminting/republishing.
   * Local preview / "You are sharing" stay up; show a spinner overlay.
   */
  republishing: boolean;
  remoteStream: MediaStream | null;
  /** LiveKit remote video for track.attach() in the spotlight. */
  remoteVideoTrack: RemoteTrack | null;
  localPreviewStream: MediaStream | null;
  currentSharer: string | null;
  error: string | null;
  shareBlocked: boolean;
  startShare: () => Promise<void>;
  stopShare: () => Promise<void>;
  clearError: () => void;
};

/** User dismissed getDisplayMedia, or LiveKit aborted a superseded connect. */
function isBenignScreenShareAbort(error: unknown): boolean {
  if (error == null) return false;
  if (typeof DOMException !== "undefined" && error instanceof DOMException) {
    if (error.name === "AbortError") return true;
  }
  if (error instanceof Error) {
    if (error.name === "AbortError") return true;
    const msg = error.message.toLowerCase();
    if (msg.includes("operation was aborted")) return true;
    if (msg.includes("connection attempt aborted")) return true;
    if (msg.includes("client initiated disconnect")) return true;
    if (msg.includes("signal connection aborted")) return true;
    // LiveKit ConnectionError.cancelled(...)
    if (
      error.name === "ConnectionError" &&
      (msg.includes("abort") || msg.includes("cancel"))
    ) {
      return true;
    }
  }
  return false;
}

/**
 * LiveKit peer-connection / ICE blips after a successful mint. Share stays
 * available; do not raise a blocking red banner (publish may still work).
 */
function isTransientPcConnectionError(error: unknown): boolean {
  if (error == null || !(error instanceof Error)) return false;
  const msg = error.message.toLowerCase();
  if (msg.includes("could not establish pc connection")) return true;
  if (msg.includes("pc connection")) return true;
  if (msg.includes("peerconnection")) return true;
  if (msg.includes("ice connection")) return true;
  if (msg.includes("ice failed")) return true;
  return false;
}

function shouldSuppressScreenShareError(error: unknown): boolean {
  return isBenignScreenShareAbort(error) || isTransientPcConnectionError(error);
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

const REPUBLISH_BACKOFF_MS = [500, 1000, 2000, 4000, 8000] as const;

/**
 * Connects a LiveKit subscriber when a huddle is active and LiveKit is
 * configured. Publishes at most one local screen track.
 *
 * Share stays visible after a PC/connect failure: only relay
 * `screen_share_unavailable` hides the control. A connect generation
 * rejects stale subscriber errors when startShare takes over the room.
 * Benign AbortErrors and transient PC/ICE blips are logged only; never
 * shown via setError. stopShare always restores Share availability.
 *
 * Mid-share LiveKit outages keep local preview / "You are sharing" and
 * loop remint+republish until success or the user Stops — never force a
 * second Share click as the primary recovery.
 */
export function useHuddleScreenShare(args: {
  active: boolean;
  channelId: string | null | undefined;
  parentChannelId: string | null | undefined;
  selfPubkey: string | null | undefined;
}): HuddleScreenShareState {
  const { active, channelId, parentChannelId, selfPubkey } = args;
  const [available, setAvailable] = React.useState<boolean | null>(null);
  const [connecting, setConnecting] = React.useState(false);
  const [sharing, setSharing] = React.useState(false);
  const [republishing, setRepublishing] = React.useState(false);
  const [remoteStream, setRemoteStream] = React.useState<MediaStream | null>(
    null,
  );
  const [remoteVideoTrack, setRemoteVideoTrack] =
    React.useState<RemoteTrack | null>(null);
  const [localPreviewStream, setLocalPreviewStream] =
    React.useState<MediaStream | null>(null);
  const [currentSharer, setCurrentSharer] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const sessionRef = React.useRef<HuddleScreenShareSession | null>(null);
  /** Bumped on effect cleanup and when startShare claims the room. */
  const connectGenRef = React.useRef(0);
  /** Bumped to cancel an in-flight republish loop (stopShare / unmount). */
  const republishGenRef = React.useRef(0);
  const wantShareRef = React.useRef(false);
  const heldStreamRef = React.useRef<MediaStream | null>(null);
  const channelRef = React.useRef(channelId);
  channelRef.current = channelId;
  const parentRef = React.useRef(parentChannelId);
  parentRef.current = parentChannelId;
  const selfPubkeyRef = React.useRef(selfPubkey);
  selfPubkeyRef.current = selfPubkey;

  const softResubscribeRef = React.useRef<(() => void) | null>(null);
  const scheduleRepublishRef = React.useRef<(() => void) | null>(null);

  const clearHeldShare = React.useCallback(() => {
    wantShareRef.current = false;
    heldStreamRef.current = null;
    setRepublishing(false);
  }, []);

  const sessionCallbacks = React.useMemo(
    () => ({
      onRemoteChanged: (remote: ScreenShareRemote | null) => {
        setRemoteStream(remote?.stream ?? null);
        setRemoteVideoTrack(remote?.videoTrack ?? null);
      },
      onLocalPreviewChanged: (stream: MediaStream | null) => {
        // Non-null: session owns a published/held local track. Null during
        // reconnect must not clear an early pre-connect preview — only
        // stopShare / failed acquire clears React preview state.
        if (stream) {
          heldStreamRef.current = stream;
          wantShareRef.current = true;
          setLocalPreviewStream(stream);
          setSharing(true);
        }
      },
      onCurrentSharerChanged: (pubkey: string | null) => {
        setCurrentSharer(pubkey);
      },
      onDisconnected: () => {
        // Permanent LiveKit disconnect with no local capture — quiet
        // subscriber resubscribe; no red banner.
        softResubscribeRef.current?.();
      },
      onPublishInterrupted: () => {
        // SFU died mid-share: keep preview, spin overlay, remint+republish.
        scheduleRepublishRef.current?.();
      },
      onReconnectingChanged: (active: boolean) => {
        if (!wantShareRef.current) return;
        setRepublishing(active);
      },
    }),
    [],
  );

  const runRepublishLoop = React.useCallback(async () => {
    const gen = ++republishGenRef.current;
    setRepublishing(true);
    let attempt = 0;
    while (
      gen === republishGenRef.current &&
      wantShareRef.current &&
      heldStreamRef.current &&
      channelRef.current
    ) {
      const stream = heldStreamRef.current;
      const [track] = stream.getVideoTracks();
      if (!track || track.readyState === "ended") {
        // OS picker ended capture — abandon without forcing Share again UI
        // (stopShare path / track ended already clears).
        break;
      }
      try {
        const minted = await mintScreenShareToken({
          channelId: channelRef.current,
          parentChannelId: parentRef.current,
          intent: "publish",
        });
        if (gen !== republishGenRef.current || !wantShareRef.current) return;
        if ("unavailable" in minted && minted.unavailable) {
          setAvailable(false);
          setError(minted.reason);
          break;
        }
        const token = minted as ScreenTokenResponse;
        setAvailable(true);
        setCurrentSharer(token.current_sharer ?? selfPubkeyRef.current ?? null);
        connectGenRef.current += 1;
        let session = sessionRef.current;
        if (!session) {
          session = new HuddleScreenShareSession(sessionCallbacks);
          sessionRef.current = session;
        } else {
          await session.detachRoomKeepLocal();
        }
        if (gen !== republishGenRef.current || !wantShareRef.current) return;
        await session.connect(token.url, token.token);
        if (gen !== republishGenRef.current || !wantShareRef.current) return;
        await session.startShare(stream);
        if (gen !== republishGenRef.current || !wantShareRef.current) return;
        setSharing(true);
        setLocalPreviewStream(stream);
        setError(null);
        setRepublishing(false);
        return;
      } catch (e) {
        if (gen !== republishGenRef.current || !wantShareRef.current) return;
        console.debug("[huddle] screen-share republish attempt failed", e);
        const delay =
          REPUBLISH_BACKOFF_MS[
            Math.min(attempt, REPUBLISH_BACKOFF_MS.length - 1)
          ]!;
        attempt += 1;
        await sleep(delay);
      }
    }
    if (gen === republishGenRef.current) {
      setRepublishing(false);
    }
  }, [sessionCallbacks]);

  React.useEffect(() => {
    scheduleRepublishRef.current = () => {
      if (!wantShareRef.current || !heldStreamRef.current) return;
      void runRepublishLoop();
    };
    return () => {
      scheduleRepublishRef.current = null;
    };
  }, [runRepublishLoop]);

  const teardown = React.useCallback(async () => {
    republishGenRef.current += 1;
    clearHeldShare();
    const session = sessionRef.current;
    sessionRef.current = null;
    if (session) {
      await session.disconnect();
    }
    const channel = channelRef.current;
    if (channel) {
      await stopScreenShareSlot({
        channelId: channel,
        parentChannelId: parentRef.current,
      });
    }
    setSharing(false);
    setRemoteStream(null);
    setRemoteVideoTrack(null);
    setLocalPreviewStream(null);
    setCurrentSharer(null);
    setConnecting(false);
    setRepublishing(false);
  }, [clearHeldShare]);

  React.useEffect(() => {
    let cancelled = false;
    const gen = ++connectGenRef.current;

    async function connectSubscriber() {
      if (!active || !channelId) {
        await teardown();
        setAvailable(null);
        return;
      }
      setConnecting(true);
      setError(null);
      try {
        const minted = await mintScreenShareToken({
          channelId,
          parentChannelId: parentRef.current,
          intent: "subscribe",
        });
        if (cancelled || gen !== connectGenRef.current) return;
        if ("unavailable" in minted && minted.unavailable) {
          setAvailable(false);
          setConnecting(false);
          return;
        }
        // Mint succeeded → LiveKit is configured. Keep Share visible even if
        // the peer connection later times out (user can retry via Share).
        setAvailable(true);
        const token = minted as ScreenTokenResponse;
        setCurrentSharer(token.current_sharer);
        // startShare may have installed a publish session while we minted.
        if (sessionRef.current) {
          return;
        }
        const session = new HuddleScreenShareSession(sessionCallbacks);
        sessionRef.current = session;
        await session.connect(token.url, token.token);
        if (cancelled || gen !== connectGenRef.current) return;
      } catch (e) {
        if (cancelled || gen !== connectGenRef.current) return;
        // Aborts + transient PC/ICE: keep Share, no scary banner.
        if (shouldSuppressScreenShareError(e)) {
          console.debug("[huddle] screen-share subscriber connect soft-fail", e);
          if (sessionRef.current && !sessionRef.current.isConnected) {
            const dead = sessionRef.current;
            sessionRef.current = null;
            void dead.disconnect();
          }
          return;
        }
        // PC/auth/network failures must not hide Share — only true unavailable
        // above does. Stale subscriber rejects after startShare reconnect are
        // ignored via connectGenRef.
        setError(e instanceof Error ? e.message : String(e));
        if (sessionRef.current && !sessionRef.current.isConnected) {
          const dead = sessionRef.current;
          sessionRef.current = null;
          void dead.disconnect();
        }
      } finally {
        if (!cancelled && gen === connectGenRef.current) {
          setConnecting(false);
        }
      }
    }

    softResubscribeRef.current = () => {
      if (cancelled) return;
      // Never clobber an in-progress local share / republish.
      if (wantShareRef.current || heldStreamRef.current) {
        scheduleRepublishRef.current?.();
        return;
      }
      // Drop the dead session handle without flipping available/sharing UI
      // into an error state, then reconnect the subscriber quietly.
      const dead = sessionRef.current;
      sessionRef.current = null;
      if (dead) {
        void dead.disconnect();
      }
      const gen = ++connectGenRef.current;
      void (async () => {
        if (!channelRef.current) return;
        try {
          const minted = await mintScreenShareToken({
            channelId: channelRef.current,
            parentChannelId: parentRef.current,
            intent: "subscribe",
          });
          if (cancelled || gen !== connectGenRef.current) return;
          if ("unavailable" in minted && minted.unavailable) {
            setAvailable(false);
            return;
          }
          setAvailable(true);
          if (sessionRef.current) return;
          const token = minted as ScreenTokenResponse;
          const session = new HuddleScreenShareSession(sessionCallbacks);
          sessionRef.current = session;
          await session.connect(token.url, token.token);
        } catch (e) {
          if (cancelled || gen !== connectGenRef.current) return;
          if (shouldSuppressScreenShareError(e)) {
            console.debug("[huddle] screen-share soft-resubscribe blip", e);
            return;
          }
          console.debug("[huddle] screen-share soft-resubscribe failed", e);
        }
      })();
    };

    void connectSubscriber();
    return () => {
      cancelled = true;
      softResubscribeRef.current = null;
      connectGenRef.current += 1;
      republishGenRef.current += 1;
      void teardown();
    };
    // parentChannelId is read via parentRef so parent filling in after join
    // does not tear down a healthy LiveKit PC and hide Share on reconnect.
  }, [active, channelId, sessionCallbacks, teardown]);

  const startShare = React.useCallback(async () => {
    if (!channelId) return;
    setError(null);
    // First await MUST be getDisplayMedia — WebKit/WKWebView needs a user gesture.
    let acquired: MediaStream | null = null;
    try {
      acquired = await acquireDisplayMedia();
      // Local preview is independent of LiveKit publish — show it immediately
      // even if mint/connect later fails / needs republish.
      heldStreamRef.current = acquired;
      wantShareRef.current = true;
      setLocalPreviewStream(acquired);
      setSharing(true);
      const minted = await mintScreenShareToken({
        channelId,
        parentChannelId: parentRef.current,
        intent: "publish",
      });
      if ("unavailable" in minted && minted.unavailable) {
        stopMediaStreamTracks(acquired);
        acquired = null;
        heldStreamRef.current = null;
        wantShareRef.current = false;
        setLocalPreviewStream(null);
        setSharing(false);
        setAvailable(false);
        setError(minted.reason);
        return;
      }
      const token = minted as ScreenTokenResponse;
      setAvailable(true);
      setCurrentSharer(token.current_sharer ?? selfPubkey ?? null);
      // Invalidate in-flight subscriber connect so its catch cannot hide Share
      // after we reclaim the same session for publish.
      connectGenRef.current += 1;
      let session = sessionRef.current;
      if (!session) {
        session = new HuddleScreenShareSession(sessionCallbacks);
        sessionRef.current = session;
      }
      // Reconnect with the publish-capable token — subscribe JWTs cannot publish.
      await session.connect(token.url, token.token);
      await session.startShare(acquired);
      acquired = null; // ownership transferred to session
      setSharing(true);
      // Publish succeeded — drop any stale subscriber PC banner.
      setError(null);
      setRepublishing(false);
    } catch (e) {
      // Picker dismiss before we held a stream — quiet exit.
      if (!heldStreamRef.current || !wantShareRef.current) {
        stopMediaStreamTracks(acquired);
        if (shouldSuppressScreenShareError(e)) {
          console.debug("[huddle] screen-share start soft-fail", e);
          setAvailable(true);
          return;
        }
        setError(e instanceof Error ? e.message : String(e));
        setLocalPreviewStream(null);
        setSharing(false);
        return;
      }
      // Capture is live but mint/connect/publish failed — keep "You are
      // sharing" preview and continually republish until Stop or success.
      acquired = null;
      if (shouldSuppressScreenShareError(e)) {
        console.debug(
          "[huddle] screen-share start soft-fail → republish loop",
          e,
        );
      } else {
        console.debug(
          "[huddle] screen-share start failed → republish loop",
          e,
        );
      }
      setAvailable(true);
      setSharing(true);
      scheduleRepublishRef.current?.();
    }
  }, [channelId, selfPubkey, sessionCallbacks]);

  const stopShare = React.useCallback(async () => {
    setError(null);
    republishGenRef.current += 1;
    wantShareRef.current = false;
    heldStreamRef.current = null;
    setRepublishing(false);
    try {
      await sessionRef.current?.stopShare();
      if (channelId) {
        await stopScreenShareSlot({
          channelId,
          parentChannelId: parentRef.current,
        });
      }
    } catch (e) {
      // Slot/stop failures must not permanently hide Share.
      if (!shouldSuppressScreenShareError(e)) {
        setError(e instanceof Error ? e.message : String(e));
      } else {
        console.debug("[huddle] screen-share stop soft-fail", e);
      }
    } finally {
      setCurrentSharer(null);
      setLocalPreviewStream(null);
      setSharing(false);
      setRepublishing(false);
      // After stop, Share must return unless relay reported true unavailable.
      setAvailable((prev) => (prev === false ? false : true));
    }
  }, [channelId]);

  const shareBlocked = isShareBlockedByOther({
    selfPubkey,
    currentSharer: sharing ? selfPubkey : currentSharer,
  });

  return {
    available,
    connecting,
    sharing,
    republishing,
    remoteStream,
    remoteVideoTrack,
    localPreviewStream,
    currentSharer,
    error,
    shareBlocked,
    startShare,
    stopShare,
    clearError: () => setError(null),
  };
}
