import 'dart:convert';
import 'dart:isolate';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../crypto/event_signing.dart';
import '../relay/nostr_models.dart';

/// Whether [event]'s id and BIP-340 signature are valid. The relay session
/// does not verify on receive, so every projection must pass this first.
bool hasVerifiedEventProof(NostrEvent event) {
  try {
    verifiedNostrEventFromJson(jsonEncode(event.toJson()));
    return true;
  } catch (_) {
    return false;
  }
}

/// Checks a batch of events and returns one result per event, in order.
typedef VerifiedProfileProofChecker =
    Future<List<bool>> Function(List<NostrEvent> events);

List<bool> _checkAll(List<NostrEvent> events) => [
  for (final event in events) hasVerifiedEventProof(event),
];

/// Production checker: one isolate per batch keeps Schnorr work off the UI
/// thread. Tests override it with a synchronous checker.
final verifiedProfileProofCheckerProvider =
    Provider<VerifiedProfileProofChecker>(
      (ref) =>
          (events) => Isolate.run(() => _checkAll(events)),
    );
