import 'package:nostr/nostr.dart' as nostr;

/// Truncates a hex pubkey to the first 8 characters with an ellipsis.
String shortPubkey(String pubkey) {
  if (pubkey.length > 12) return '${pubkey.substring(0, 8)}\u2026';
  return pubkey;
}

/// Compact display form keeping both ends of a key: `abcd1234…wxyz`.
///
/// Mirrors desktop's `truncatePubkey` (desktop/src/shared/lib/pubkey.ts). The
/// trailing characters matter: bech32 npubs all start with `npub1`, so a
/// head-only form like [shortPubkey] leaves barely three distinguishing
/// characters. A truncated key is a recognition aid, never an identity proof
/// — vanity grinders forge short prefixes cheaply.
String truncatePubkey(String pubkey) {
  if (pubkey.length <= 12) return pubkey;
  final head = pubkey.substring(0, 8);
  final tail = pubkey.substring(pubkey.length - 4);
  return '$head\u2026$tail';
}

/// Encode a hex pubkey as a bech32 `npub1…`, or null if it is not encodable.
///
/// Mirrors desktop's `safeNpub` (desktop/src/shared/lib/nostrUtils.ts): never
/// throws, so display code can fall back rather than crash on malformed input.
String? safeNpub(String pubkey) {
  try {
    return nostr.Bech32Entity.encode(
      prefix: nostr.Nip19Prefix.npub,
      data: pubkey,
    );
  } catch (_) {
    return null;
  }
}
