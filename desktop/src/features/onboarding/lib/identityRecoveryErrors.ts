/**
 * Maps backend pairing failures to copy the user can act on.
 *
 * Connection failures are called out with the relay address when it is known:
 * a fresh desktop still pointed at the built-in `ws://localhost:3000` default
 * fails this way instantly (block/buzz#5236), and a generic "expired" message
 * sends people looking at the wrong thing. Everything else keeps the existing
 * expired/lost-connection wording.
 */
export function recoveryErrorMessage(
  message: string,
  pairingRelayUrl: string | null = null,
): string {
  const normalized = message.toLowerCase();
  if (isConnectionFailure(normalized) && pairingRelayUrl) {
    return `Could not connect to the pairing relay at ${pairingRelayUrl}. Check the community address this desktop is configured with, then create a new code.`;
  }
  if (
    normalized.includes("sas-confirm") ||
    isConnectionFailure(normalized) ||
    normalized.includes("expired") ||
    normalized.includes("timed out")
  ) {
    return "This pairing code expired or lost its connection. Create a new code and try again.";
  }
  return message;
}

function isConnectionFailure(normalized: string): boolean {
  return (
    normalized.includes("relay connection closed") ||
    normalized.includes("websocket") ||
    normalized.includes("connection refused") ||
    normalized.includes("failed to connect")
  );
}
