import '../../shared/relay/nostr_models.dart';

/// Attribution is authoritative only on a message signed by NIP-11 `self`.
/// Match relay ingest, including its compatibility fallback for legacy posts.
String publisherMessageAuthor(NostrEvent event, String? relayAuthor) {
  if (relayAuthor == null || event.pubkey != relayAuthor) return event.pubkey;
  for (final key in ['actor', 'p']) {
    for (final tag in event.tags) {
      if (tag.length >= 2 &&
          tag[0] == key &&
          RegExp(r'^[0-9a-f]{64}$').hasMatch(tag[1])) {
        return tag[1];
      }
    }
  }
  return event.pubkey;
}
