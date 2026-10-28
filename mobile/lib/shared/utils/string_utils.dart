import 'package:nostr/nostr.dart' as nostr;

/// Neutral label rendered when an identity string cannot be read as a Nostr
/// public key — malformed identities never leak truncated raw hex into the UI.
const String unknownIdentityLabel = 'Unknown identity';

final RegExp _hexPubkeyPattern = RegExp(r'^[0-9a-fA-F]{64}$');

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
    if (_hexPubkeyPattern.hasMatch(trimmed)) {
      return _encodeNpub(trimmed.toLowerCase());
    }
    final decoded = nostr.Nip19.decode(payload: trimmed);
    if (decoded.prefix != nostr.Nip19Prefix.npub ||
        !_hexPubkeyPattern.hasMatch(decoded.data)) {
      return null;
    }
    return _encodeNpub(decoded.data.toLowerCase());
  } catch (_) {
    return null;
  }
}

String _encodeNpub(String hexPubkey) =>
    nostr.Nip19.encode(prefix: nostr.Nip19Prefix.npub, data: hexPubkey);

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
