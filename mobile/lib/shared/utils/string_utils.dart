import 'package:flutter/foundation.dart' show visibleForTesting;
import 'package:nostr/nostr.dart' as nostr;

/// Neutral label rendered when an identity string cannot be read as a Nostr
/// public key — malformed identities never leak truncated raw hex into the UI.
const String unknownIdentityLabel = 'Unknown identity';

/// Whether [value] is a 64-character hex public key (any case).
///
/// A character loop rather than a regular expression: AOT builds on iOS run
/// regular expressions in an interpreter, and identity views check every key
/// of a large roster on each rebuild.
bool isHexPubkey(String value) {
  if (value.length != 64) return false;
  for (var i = 0; i < 64; i++) {
    final c = value.codeUnitAt(i);
    if (!((c >= 0x30 && c <= 0x39) || // 0-9
        (c >= 0x61 && c <= 0x66) || // a-f
        (c >= 0x41 && c <= 0x46))) {
      // A-F
      return false;
    }
  }
  return true;
}

/// The canonical full NIP-19 npub for [identity], or null when [identity] is
/// not a valid public key.
///
/// Accepts both canonical public-key forms — a 64-character hex key (any
/// case) or an `npub1…` string — and returns the full canonical npub.
/// Already-npub input round-trips through the codec so the result is always
/// the canonical lowercase encoding. Internal storage stays hex; the npub is
/// the user-facing display/copy form. Invalid input returns null (never a
/// passthrough of the raw string) so callers can suppress copy actions.
String? fullNpub(String identity) {
  final trimmed = identity.trim();
  try {
    if (isHexPubkey(trimmed)) {
      return _encodeNpub(trimmed.toLowerCase());
    }
    final decoded = nostr.Nip19.decode(payload: trimmed);
    if (decoded.prefix != nostr.Nip19Prefix.npub ||
        !isHexPubkey(decoded.data)) {
      return null;
    }
    return _encodeNpub(decoded.data.toLowerCase());
  } catch (_) {
    return null;
  }
}

/// Encoded npubs by lowercase hex key. Identity views label every roster
/// member without a profile name by npub on each rebuild, and bech32 encoding
/// dominated those rebuilds on large channels. Bounded: once full it keeps
/// what it has, so a roster larger than the limit still reuses most keys.
final _npubs = <String, String>{};
@visibleForTesting
const npubCacheLimit = 8192;

String _encodeNpub(String hexPubkey) {
  final cached = _npubs[hexPubkey];
  if (cached != null) return cached;
  final npub = nostr.Nip19.encode(
    prefix: nostr.Nip19Prefix.npub,
    data: hexPubkey,
  );
  if (_npubs.length < npubCacheLimit) _npubs[hexPubkey] = npub;
  return npub;
}

/// The canonical compact public-key label: the first 8 and last 4 characters
/// of the full npub joined by an ellipsis — the same truncation desktop uses
/// for identity surfaces.
///
/// A compact label is a recognition aid, never a verification path; copy
/// actions expose the full npub via [fullNpub]. Accepts hex or npub input
/// (already-npub input yields the same label as its underlying key).
/// Returns [unknownIdentityLabel] when [identity] is not a valid public key.
String shortPubkey(String identity) {
  final npub = fullNpub(identity);
  if (npub == null) return unknownIdentityLabel;
  return npub.length <= 12
      ? npub
      : '${npub.substring(0, 8)}…${npub.substring(npub.length - 4)}';
}
