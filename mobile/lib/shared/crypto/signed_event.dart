import 'package:nostr/nostr.dart' as nostr;

import '../relay/nostr_models.dart';

/// Verify the canonical event id and author's signature without a wall-clock
/// freshness restriction. Authority readers apply their own kind/signer scope.
bool verifySignedEvent(NostrEvent event) {
  if (!RegExp(r'^[0-9a-f]{64}$').hasMatch(event.pubkey) ||
      !RegExp(r'^[0-9a-f]{64}$').hasMatch(event.id) ||
      !RegExp(r'^[0-9a-f]{128}$').hasMatch(event.sig) ||
      event.createdAt < 0 ||
      event.kind < 0 ||
      event.kind > 65535) {
    return false;
  }
  try {
    final signed = nostr.Event.fromMap(event.toJson(), verify: false);
    return signed.getEventId() == event.id &&
        nostr.Schnorr.verify(
          publicKey: event.pubkey,
          message: event.id,
          signature: event.sig,
        );
  } catch (_) {
    return false;
  }
}
