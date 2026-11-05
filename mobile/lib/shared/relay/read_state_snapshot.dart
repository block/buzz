import 'dart:convert';

import 'package:flutter/foundation.dart';

import 'package:nostr/nostr.dart' as nostr;

import 'nostr_models.dart';
import 'relay_query_response.dart';

/// Snapshot bytes and the authenticated scope they must match.
typedef ReadStateSnapshotInput = ({
  Uint8List bytes,
  String? contentType,
  String viewer,
  String community,
});

/// Decodes and validates a complete snapshot.
typedef ReadStateSnapshotDecoder =
    Future<List<NostrEvent>> Function(ReadStateSnapshotInput input);

/// Keeps snapshot parsing and Schnorr verification off the UI isolate.
Future<List<NostrEvent>> decodeReadStateSnapshot(
  ReadStateSnapshotInput input,
) => compute(validateReadStateSnapshot, input);

/// Hard bounds of the version-1 atomic snapshot protocol.
const readStateSnapshotMaxBytes = 8 * 1024 * 1024;

/// The events plus room for the snapshot envelope around them.
const readStateSnapshotMaxEnvelopeBytes = readStateSnapshotMaxBytes + 4096;
const readStateSnapshotMaxEvents = 4096;
final _hex64 = RegExp(r'^[0-9a-f]{64}$');
final _hex128 = RegExp(r'^[0-9a-f]{128}$');
final _uuid = RegExp(
  r'^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$',
);

/// Requires explicit version-1 host-bound NIP-11 discovery, never a fallback.
String readStateSnapshotCommunity(Object? descriptor) {
  if (descriptor is! Map<String, dynamic> ||
      descriptor['version'] != 1 ||
      descriptor['community_id'] is! String ||
      !_uuid.hasMatch(descriptor['community_id'] as String)) {
    throw const FormatException('Atomic read-state snapshot not discovered');
  }
  return descriptor['community_id'] as String;
}

/// Validates the entire snapshot atomically, on a worker isolate via compute.
/// No decryption or read-state namespace selection occurs in this transport.
List<NostrEvent> validateReadStateSnapshot(ReadStateSnapshotInput input) {
  final raw = jsonDecode(
    relayResponseText(
      input.bytes,
      input.contentType,
      maxInflatedBytes: readStateSnapshotMaxEnvelopeBytes,
    ),
  );
  if (raw is! Map<String, dynamic> ||
      raw['read_state_snapshot'] != 1 ||
      raw['complete'] != true ||
      raw['pubkey'] != input.viewer ||
      raw['community_id'] != input.community ||
      raw['snapshot_id'] is! String ||
      !_hex64.hasMatch(raw['snapshot_id'] as String) ||
      raw['events'] is! List ||
      (raw['events'] as List).length > readStateSnapshotMaxEvents ||
      utf8.encode(jsonEncode(raw['events'])).length >
          readStateSnapshotMaxBytes) {
    throw const FormatException('Incomplete or mismatched read-state snapshot');
  }
  final coordinates = <String>{};
  final events = <NostrEvent>[];
  for (final value in raw['events'] as List) {
    try {
      if (value is! Map<String, dynamic>) {
        throw const FormatException('Invalid event object');
      }
      final event = NostrEvent.fromJson(value);
      if (event.pubkey != input.viewer ||
          event.kind != 30078 ||
          !_hex64.hasMatch(event.id) ||
          !_hex64.hasMatch(event.pubkey) ||
          !_hex128.hasMatch(event.sig) ||
          event.createdAt < 0 ||
          event.createdAt > 9007199254740991) {
        throw const FormatException('Invalid snapshot event envelope');
      }
      // NIP-33 uses the first d tag, or the empty identifier if absent.
      var coordinate = '';
      for (final tag in event.tags) {
        if (tag.isNotEmpty && tag.first == 'd') {
          coordinate = tag.length > 1 ? tag[1] : '';
          break;
        }
      }
      if (!coordinates.add(coordinate)) {
        throw const FormatException('Duplicate snapshot coordinate');
      }
      // The package's verify:true additionally rejects timestamp zero, which
      // NIP-01 permits. Enforce wire shape above, canonical hash and signature
      // here without imposing that unrelated timestamp policy.
      final signed = nostr.Event.fromMap(value, verify: false);
      if (signed.getEventId() != event.id ||
          !nostr.Schnorr.verify(
            publicKey: event.pubkey,
            message: event.id,
            signature: event.sig,
          )) {
        throw const FormatException('Invalid snapshot event signature');
      }
      events.add(event);
    } catch (_) {
      throw const FormatException('Invalid or duplicate snapshot event');
    }
  }
  return events;
}
