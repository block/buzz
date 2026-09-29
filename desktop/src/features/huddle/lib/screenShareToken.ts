import { invokeTauri } from "@/shared/api/tauri";

export type ScreenShareIntent = "subscribe" | "publish";

export type ScreenTokenResponse = {
  url: string;
  token: string;
  room: string;
  can_publish: boolean;
  current_sharer: string | null;
  expires_in_secs?: number;
};

export type ScreenShareUnavailable = {
  unavailable: true;
  reason: string;
};

export function isScreenShareUnavailableError(error: unknown): boolean {
  if (!(error instanceof Error)) return false;
  const msg = error.message.toLowerCase();
  return (
    msg.includes("screen_share_unavailable") ||
    msg.includes("livekit is not configured")
  );
}

export function isScreenShareBusyError(error: unknown): boolean {
  if (!(error instanceof Error)) return false;
  return error.message.toLowerCase().includes("screen_share_busy");
}

/**
 * Mint a short-lived LiveKit token for huddle screen share.
 * Goes through Tauri/reqwest (NIP-98) so browser CORS does not apply.
 * Returns `unavailable` when the relay has no LiveKit config (503).
 */
export async function mintScreenShareToken(args: {
  channelId: string;
  parentChannelId?: string | null;
  intent: ScreenShareIntent;
}): Promise<ScreenTokenResponse | ScreenShareUnavailable> {
  return invokeTauri<ScreenTokenResponse | ScreenShareUnavailable>(
    "huddle_screen_token",
    {
      channelId: args.channelId,
      parentChannelId: args.parentChannelId ?? null,
      intent: args.intent,
    },
  );
}

/** Best-effort release of the relay soft sharer lock. */
export async function stopScreenShareSlot(args: {
  channelId: string;
  parentChannelId?: string | null;
}): Promise<void> {
  try {
    await invokeTauri("huddle_screen_stop", {
      channelId: args.channelId,
      parentChannelId: args.parentChannelId ?? null,
    });
  } catch {
    /* best-effort */
  }
}
