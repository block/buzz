import 'dart:convert';
import 'dart:isolate';

import 'package:buzz/shared/crypto/event_signing.dart';

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
      verifiedNostrEventFromJson(jsonEncode(event.toJson()));
    } catch (_) {
      // A corrupt record must not hide unrelated verified agents.
      continue;
    }
    final previous = heads[id];
    if (previous == null || meEventIsNewer(event, previous)) heads[id] = event;
  }
  // Empty/deleted, malformed and unsupported heads never resurrect an earlier
  // listing.
  return {
    for (final MapEntry(key: id, value: head) in heads.entries)
      id: ?_supportedPolicyName(head),
  };
}

/// Desktop's `ManagedAgentEventContent` contract: a head desktop cannot parse
/// fails closed instead of offering an agent whose policy is unknown.
String? _supportedPolicyName(NostrEvent event) {
  final Object? decoded;
  try {
    decoded = jsonDecode(event.content);
  } on FormatException {
    return null;
  }
  if (decoded is! Map<String, dynamic>) return null;
  final body = decoded;
  final name = body['name'];
  if (name is! String || name.trim().isEmpty) return null;
  final parallelism = body['parallelism'];
  final allowlist = body['respond_to_allowlist'];
  final supported =
      parallelism is int &&
      parallelism >= 0 &&
      parallelism <= 0xFFFFFFFF &&
      _respondTo.contains(body['respond_to']) &&
      (!body.containsKey('respond_to_allowlist') ||
          (allowlist is List && allowlist.every((entry) => entry is String))) &&
      _optionalPolicyText.every((field) {
        final value = body[field];
        return value == null || value is String;
      });
  return supported ? name.trim() : null;
}

const _respondTo = {'owner-only', 'allowlist', 'anyone'};
const _optionalPolicyText = [
  'persona_id',
  'system_prompt',
  'model',
  'provider',
  'persona_source_version',
];
