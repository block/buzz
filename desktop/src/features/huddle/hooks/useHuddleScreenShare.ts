import * as React from "react";

import {
  HuddleScreenShareSession,
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
  remoteStream: MediaStream | null;
  localPreviewStream: MediaStream | null;
  currentSharer: string | null;
  error: string | null;
  shareBlocked: boolean;
  startShare: () => Promise<void>;
  stopShare: () => Promise<void>;
  clearError: () => void;
};

/**
 * Connects a LiveKit subscriber when a huddle is active and LiveKit is
 * configured. Publishes at most one local screen track.
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
  const [remoteStream, setRemoteStream] = React.useState<MediaStream | null>(
    null,
  );
  const [localPreviewStream, setLocalPreviewStream] =
    React.useState<MediaStream | null>(null);
  const [currentSharer, setCurrentSharer] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const sessionRef = React.useRef<HuddleScreenShareSession | null>(null);
  const channelRef = React.useRef(channelId);
  channelRef.current = channelId;
  const parentRef = React.useRef(parentChannelId);
  parentRef.current = parentChannelId;

  const teardown = React.useCallback(async () => {
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
    setLocalPreviewStream(null);
    setCurrentSharer(null);
    setConnecting(false);
  }, []);

  React.useEffect(() => {
    let cancelled = false;

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
          parentChannelId,
          intent: "subscribe",
        });
        if (cancelled) return;
        if ("unavailable" in minted && minted.unavailable) {
          setAvailable(false);
          setConnecting(false);
          return;
        }
        setAvailable(true);
        const token = minted as ScreenTokenResponse;
        setCurrentSharer(token.current_sharer);
        const session = new HuddleScreenShareSession({
          onRemoteChanged: (remote: ScreenShareRemote | null) => {
            setRemoteStream(remote?.stream ?? null);
          },
          onLocalPreviewChanged: (stream) => {
            setLocalPreviewStream(stream);
            setSharing(stream != null);
          },
          onCurrentSharerChanged: (pubkey) => {
            setCurrentSharer(pubkey);
          },
        });
        sessionRef.current = session;
        await session.connect(token.url, token.token);
      } catch (e) {
        if (cancelled) return;
        // Treat network/auth failures as unavailable for the Share button.
        setAvailable(false);
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (!cancelled) setConnecting(false);
      }
    }

    void connectSubscriber();
    return () => {
      cancelled = true;
      void teardown();
    };
  }, [active, channelId, parentChannelId, teardown]);

  const startShare = React.useCallback(async () => {
    if (!channelId) return;
    setError(null);
    try {
      const minted = await mintScreenShareToken({
        channelId,
        parentChannelId,
        intent: "publish",
      });
      if ("unavailable" in minted && minted.unavailable) {
        setAvailable(false);
        setError(minted.reason);
        return;
      }
      const token = minted as ScreenTokenResponse;
      setCurrentSharer(token.current_sharer ?? selfPubkey ?? null);
      // Reconnect with the publish-capable token — subscribe JWTs cannot publish.
      let session = sessionRef.current;
      if (!session) {
        session = new HuddleScreenShareSession({
          onRemoteChanged: (remote) => setRemoteStream(remote?.stream ?? null),
          onLocalPreviewChanged: (stream) => {
            setLocalPreviewStream(stream);
            setSharing(stream != null);
          },
          onCurrentSharerChanged: setCurrentSharer,
        });
        sessionRef.current = session;
      }
      await session.connect(token.url, token.token);
      await session.startShare();
      setSharing(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setSharing(false);
    }
  }, [channelId, parentChannelId, selfPubkey]);

  const stopShare = React.useCallback(async () => {
    setError(null);
    try {
      await sessionRef.current?.stopShare();
      if (channelId) {
        await stopScreenShareSlot({
          channelId,
          parentChannelId,
        });
      }
      setCurrentSharer(null);
      setSharing(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [channelId, parentChannelId]);

  const shareBlocked = isShareBlockedByOther({
    selfPubkey,
    currentSharer: sharing ? selfPubkey : currentSharer,
  });

  return {
    available,
    connecting,
    sharing,
    remoteStream,
    localPreviewStream,
    currentSharer,
    error,
    shareBlocked,
    startShare,
    stopShare,
    clearError: () => setError(null),
  };
}
