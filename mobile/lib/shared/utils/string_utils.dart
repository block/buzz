import '../nostr/nostr_keys.dart';

/// Canonical compact identity label.
///
/// Valid public keys are always rendered as npub. Keeping both ends is useful
/// because every npub begins with the same `npub1` prefix. The fallback branch
/// only supports short synthetic/test identifiers; real 64-character public
/// keys never render as hex.
String shortPubkey(String pubkey) {
  final npub = tryNpubFromPublicKey(pubkey);
  if (npub != null) return compactNpub(npub);
  if (pubkey.length > 12) return '${pubkey.substring(0, 8)}\u2026';
  return pubkey;
}

String compactNpub(String npub) {
  if (npub.length <= 24) return npub;
  return '${npub.substring(0, 12)}\u2026${npub.substring(npub.length - 8)}';
}

/// One varying npub payload character for a fallback avatar glyph.
String pubkeyAvatarInitial(String pubkey) {
  final npub = tryNpubFromPublicKey(pubkey);
  if (npub != null && npub.length > 5) return npub[5].toUpperCase();
  return pubkey.isEmpty ? '?' : pubkey[0].toUpperCase();
}

/// Event IDs are not public keys and must never be encoded as npub.
String shortEventId(String eventId) {
  if (eventId.length > 12) return '${eventId.substring(0, 8)}\u2026';
  return eventId;
}
