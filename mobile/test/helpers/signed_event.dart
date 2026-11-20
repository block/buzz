import 'package:buzz/shared/relay/relay.dart';
import 'package:nostr/nostr.dart' as nostr;

/// [event] re-signed by [keys], so it passes live read-state verification.
NostrEvent signedBy(nostr.Keys keys, NostrEvent event) {
  final signed = nostr.Event.from(
    kind: event.kind,
    tags: event.tags,
    content: event.content,
    createdAt: event.createdAt,
    secretKey: keys.secret,
  );
  return NostrEvent.fromJson(signed.toMap());
}
