import 'dart:convert';
import 'dart:isolate';

import 'package:nostr/nostr.dart' as nostr;

import '../relay/relay.dart';
import 'me_sync.dart';

/// Desktop's owner-authored agent inventory. A historical kind:0 ownership
/// attestation alone does not mean an agent is still in this inventory.
Future<Map<String, String>> loadMeAgentInventory(MeSync sync) async {
  const pageSize = 200;
  final events = <NostrEvent>[];
  int? until;
  String? beforeId;
  for (var page = 0; page < 50; page++) {
    sync.check();
    final batch = await sync.session.queryRelay([
      NostrFilter(
        kinds: const [30177],
        authors: [sync.pubkey],
        limit: pageSize,
        until: until,
        extensions: {'before_id': ?beforeId},
      ),
    ]);
    sync.check();
    if (batch.length > pageSize ||
        batch.any((e) => e.kind != 30177 || e.pubkey != sync.pubkey)) {
      throw StateError('Agent inventory response is outside its owner scope.');
    }
    events.addAll(batch);
    if (batch.length < pageSize) {
      final owner = sync.pubkey;
      return _parseInventory(events, owner);
    }
    final ordered = [...batch]
      ..sort((a, b) {
        final time = b.createdAt.compareTo(a.createdAt);
        return time != 0 ? time : a.id.compareTo(b.id);
      });
    final last = ordered.last;
    if (until != null &&
        (last.createdAt > until ||
            (last.createdAt == until && last.id.compareTo(beforeId!) <= 0))) {
      throw StateError('Agent inventory pagination did not advance.');
    }
    until = last.createdAt;
    beforeId = last.id;
  }
  throw StateError('Agent inventory exceeds the read budget.');
}

Future<Map<String, String>> _parseInventory(
  List<NostrEvent> events,
  String owner,
) => Isolate.run(() => _inventoryNames(events, owner));

Map<String, String> _inventoryNames(List<NostrEvent> events, String owner) {
  final heads = <String, NostrEvent>{};
  for (final event in events) {
    if (event.pubkey != owner) continue;
    final id = event.getTagValue('d');
    if (id == null || !RegExp(r'^[0-9a-f]{64}$').hasMatch(id)) continue;
    // Verify before allowing a newer tombstone or record to replace a head.
    try {
      nostr.Event.fromJson(jsonEncode(event.toJson()));
    } catch (_) {
      // A corrupt record must not hide unrelated verified agents.
      continue;
    }
    final previous = heads[id];
    if (previous == null || meEventIsNewer(event, previous)) heads[id] = event;
  }
  final names = <String, String>{};
  for (final entry in heads.entries) {
    try {
      final body = jsonDecode(entry.value.content);
      if (body is! Map<String, dynamic>) continue;
      final name = body['name'];
      if (name is String && name.trim().isNotEmpty) {
        names[entry.key] = name.trim();
      }
    } on FormatException {
      // Empty/deleted and malformed heads never resurrect an earlier listing.
    }
  }
  return names;
}
