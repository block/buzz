export const INVITE_EXPIRED_ERROR = "invite_expired";

/** Convert a ws(s) relay URL to its http(s) equivalent. */
export function relayHttpFromWs(wsUrl: string): string {
  if (wsUrl.startsWith("wss://")) return `https://${wsUrl.slice(6)}`;
  if (wsUrl.startsWith("ws://")) return `http://${wsUrl.slice(5)}`;
  throw new Error(`Expected ws:// or wss:// relay URL, got: ${wsUrl}`);
}

export function inviteErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : `${error}`;
}

export function isInviteExpiredError(error: unknown): boolean {
  return inviteErrorMessage(error) === INVITE_EXPIRED_ERROR;
}
