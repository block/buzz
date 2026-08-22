/// Truncates a hex pubkey to the first 8 characters with an ellipsis.
String shortPubkey(String pubkey) {
  if (pubkey.length > 12) return '${pubkey.substring(0, 8)}\u2026';
  return pubkey;
}
