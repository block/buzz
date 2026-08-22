import 'package:nostr/nostr.dart' as nostr;

/// The x-only public key for a signing key, derived at most once per key.
///
/// `Event.from` derives the pubkey itself when the caller omits it, which is a
/// second elliptic-curve multiply on top of the signature — for a value that is
/// constant per identity. On this stack that is not free: the crypto is
/// pure-Dart bigint math, and a host-AOT measurement put `Event.from` at 8.13ms
/// p50 against 5.41ms with the pubkey supplied.
///
/// Supplying it is behaviour-identical. `Event`'s id hash lowercases the pubkey
/// before hashing, and the signature covers the id, so a supplied key produces
/// the same id and the same signature as a derived one.
///
/// Caches the last key only. The app signs with one identity at a time and
/// switches on community change, so a single slot hits on every call in a run
/// of signatures without holding keys it no longer needs.
String pubkeyForPrivkey(String privkeyHex) {
  if (privkeyHex != _cachedPrivkeyHex) {
    _cachedPubkey = nostr.Schnorr.derivePublicKey(privkeyHex);
    _cachedPrivkeyHex = privkeyHex;
  }
  return _cachedPubkey;
}

String _cachedPrivkeyHex = '';
String _cachedPubkey = '';
