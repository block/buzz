import { listen } from "@tauri-apps/api/event";
import * as React from "react";

type TtsSpeakerActivity = { pubkey: string; level: number };

export function useHuddleSpeakerActivity() {
  const [remoteActiveSpeakers, setRemoteActiveSpeakers] = React.useState<
    string[]
  >([]);
  const [remoteSpeakerLevels, setRemoteSpeakerLevels] = React.useState<
    Record<string, number>
  >({});
  const [ttsSpeakerActivity, setTtsSpeakerActivity] =
    React.useState<TtsSpeakerActivity | null>(null);

  const activeSpeakers = React.useMemo(() => {
    if (!ttsSpeakerActivity) return remoteActiveSpeakers;
    const normalizedTtsPubkey = ttsSpeakerActivity.pubkey.toLowerCase();
    if (
      remoteActiveSpeakers.some(
        (pubkey) => pubkey.toLowerCase() === normalizedTtsPubkey,
      )
    ) {
      return remoteActiveSpeakers;
    }
    return [...remoteActiveSpeakers, ttsSpeakerActivity.pubkey];
  }, [remoteActiveSpeakers, ttsSpeakerActivity]);

  const speakerLevels = React.useMemo(() => {
    if (!ttsSpeakerActivity) return remoteSpeakerLevels;
    return {
      ...remoteSpeakerLevels,
      [ttsSpeakerActivity.pubkey]: Math.max(
        remoteSpeakerLevels[ttsSpeakerActivity.pubkey] ?? 0,
        ttsSpeakerActivity.level,
      ),
    };
  }, [remoteSpeakerLevels, ttsSpeakerActivity]);

  React.useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<Record<string, number>>("huddle-speaker-levels", (event) => {
      if (!cancelled) setRemoteSpeakerLevels(event.payload);
    }).then((cleanup) => {
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  React.useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<{ pubkey: string | null; level: number }>(
      "huddle-tts-speaker-level",
      (event) => {
        if (cancelled) return;
        const { pubkey, level } = event.payload;
        setTtsSpeakerActivity(
          pubkey ? { pubkey, level: Math.min(1, Math.max(0, level)) } : null,
        );
      },
    ).then((cleanup) => {
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  React.useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<string[]>("huddle-active-speakers", (event) => {
      if (!cancelled) setRemoteActiveSpeakers(event.payload);
    }).then((cleanup) => {
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const resetSpeakerActivity = React.useCallback(() => {
    setRemoteActiveSpeakers([]);
    setRemoteSpeakerLevels({});
    setTtsSpeakerActivity(null);
  }, []);

  return { activeSpeakers, resetSpeakerActivity, speakerLevels };
}
