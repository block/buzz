/**
 * CLOSED ends a NIP-01 subscription. Retry failures that may recover without
 * changing the request; authorization and malformed-filter failures require a
 * caller/state change and would otherwise loop forever.
 */
export function isRetryableRelayClosed(message: string) {
  const normalized = message.trim().toLowerCase();
  return !(
    normalized.startsWith("restricted:") ||
    normalized.startsWith("auth-required:") ||
    normalized.startsWith("blocked:") ||
    normalized.startsWith("invalid:") ||
    normalized.startsWith("pow:") ||
    normalized.startsWith("duplicate:") ||
    normalized.startsWith("unsupported:") ||
    normalized.startsWith("error: mixed search") ||
    normalized.startsWith("error: too many subscriptions")
  );
}
