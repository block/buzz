import 'dart:convert';

import 'package:nostr/nostr.dart' as nostr;

import '../crypto/nip44.dart';
import '../relay/relay.dart';
import 'default_agent.dart';
import 'me_sync.dart';

/// Publishes and confirms the owner's encrypted, community-scoped choice.
/// Failures propagate to the selection UI so the user can retry.
Future<void> saveDefaultAgent(MeSync sync, String? pubkey) async {
  if (pubkey != null && !RegExp(r'^[0-9a-f]{64}$').hasMatch(pubkey)) {
    throw const FormatException('Invalid agent public key');
  }
  sync.check();
  final filter = NostrFilter(
    kinds: const [30078],
    authors: [sync.pubkey],
    tags: {
      '#d': [defaultAgentCoordinate(sync.community)],
    },
    limit: 1,
  );
  final previous = await sync.session.fetchHistory(filter);
  sync.check();
  final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
  for (final event in previous) {
    decodeDefaultAgent(event, sync);
    if (event.createdAt >= now) {
      throw StateError('Please wait a second before saving again.');
    }
  }
  final event = NostrEvent.fromJson(
    nostr.Event.from(
      kind: 30078,
      content: nip44Encrypt(
        sync.key,
        jsonEncode({'version': 1, 'agent': pubkey}),
      ),
      secretKey: sync.privateHex,
      createdAt: now,
      tags: [
        ['d', defaultAgentCoordinate(sync.community)],
        ['t', defaultAgentTag],
      ],
      verify: false,
    ).toMap(),
  );
  await sync.session.publish(event);
  sync.check();
  final confirmed = await sync.session.fetchHistory(filter);
  sync.check();
  if (confirmed.isEmpty || confirmed.first.id != event.id) {
    throw StateError('The Bestie changed on another device. Try again.');
  }
  final prefs = sync.prefs;
  if (prefs != null &&
      !await prefs.setString(
        'main-agent:${sync.community}:${sync.pubkey}',
        jsonEncode(event.toJson()),
      )) {
    throw StateError('Bestie saved, but offline storage failed. Try again.');
  }
}
