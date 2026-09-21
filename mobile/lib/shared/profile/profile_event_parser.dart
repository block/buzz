import 'dart:isolate';

import '../crypto/nip_oa.dart';
import '../relay/nostr_models.dart';
import 'user_profile.dart';

/// A verified profile together with its NIP-01 replacement order.
typedef ParsedProfileEvent = ({
  UserProfile profile,
  int createdAt,
  String eventId,
});

/// Parses and verifies profile history without blocking the UI isolate.
///
/// Capture only the event batch here, never a provider or its surrounding
/// context. NIP-OA verification performs synchronous elliptic-curve work.
Future<List<ParsedProfileEvent>> parseProfileEventBatch(
  List<NostrEvent> events,
) => Isolate.run(() => _parseBatch(events));

List<ParsedProfileEvent> _parseBatch(List<NostrEvent> events) {
  final latest = <String, NostrEvent>{};
  for (final event in events) {
    if (event.kind != 0) continue;
    final pubkey = event.pubkey.toLowerCase();
    final current = latest[pubkey];
    if (current == null ||
        event.createdAt > current.createdAt ||
        (event.createdAt == current.createdAt &&
            event.id.compareTo(current.id) < 0)) {
      latest[pubkey] = event;
    }
  }
  return [for (final event in latest.values) parseProfileEvent(event)];
}

/// Parses a single live profile, including its verified owner attribution.
ParsedProfileEvent parseProfileEvent(NostrEvent event) {
  final data = ProfileData.fromEvent(event);
  return (
    profile: UserProfile(
      pubkey: data.pubkey.toLowerCase(),
      displayName: data.displayName,
      avatarUrl: data.avatarUrl,
      about: data.about,
      nip05Handle: data.nip05,
      ownerPubkey: verifiedOaOwnerPubkey(event.tags, event.pubkey),
    ),
    createdAt: event.createdAt,
    eventId: event.id,
  );
}
