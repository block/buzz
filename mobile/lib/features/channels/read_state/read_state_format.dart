import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../../../shared/relay/nostr_models.dart';

const readStateDTagPrefix = 'read-state:';
const readStateFetchLimit = 500;
const readStateHorizonSeconds = 7 * 24 * 60 * 60;
const _maxContexts = 10000;

typedef ReadStateDecrypt = String Function(String ciphertext);

class ReadStateBlob {
  final String clientId;
  final Map<String, int> contexts;

  ReadStateBlob({required this.clientId, required Map<String, int> contexts})
    : contexts = Map.unmodifiable(contexts);

  Map<String, dynamic> toJson() => {
    'v': 1,
    'client_id': clientId,
    'contexts': contexts,
  };
}

class DecodedReadStateEvent {
  final NostrEvent event;
  final String dTag;
  final ReadStateBlob blob;

  const DecodedReadStateEvent({
    required this.event,
    required this.dTag,
    required this.blob,
  });
}

bool isPlainJsonObject(Object? value) {
  if (value is! Map) return false;
  return value.keys.every((key) => key is String);
}

Map<String, Object?>? asStringObjectMap(Object? value) {
  if (!isPlainJsonObject(value)) return null;
  return (value as Map).cast<String, Object?>();
}

bool isValidReadStateDTag(String? value) {
  if (value == null || !value.startsWith(readStateDTagPrefix)) {
    return false;
  }

  final slotId = value.substring(readStateDTagPrefix.length);
  if (slotId.isEmpty || slotId.length > 64) {
    return false;
  }

  for (var index = 0; index < slotId.length; index++) {
    if (slotId.codeUnitAt(index) > 0x7f) {
      return false;
    }
  }
  return true;
}

bool hasValidReadStateTags(NostrEvent event) {
  final dTags = event.tags.where((tag) => tag.isNotEmpty && tag[0] == 'd');
  if (dTags.length != 1) {
    return false;
  }
  final dTag = dTags.single;
  if (dTag.length < 2 || !isValidReadStateDTag(dTag[1])) {
    return false;
  }

  final tTags = event.tags.where(
    (tag) => tag.length >= 2 && tag[0] == 't' && tag[1] == 'read-state',
  );
  return tTags.length == 1;
}

ReadStateBlob? decodeReadStateBlob(String plaintext) {
  final Object? parsed;
  try {
    parsed = jsonDecode(plaintext);
  } catch (_) {
    return null;
  }

  final record = asStringObjectMap(parsed);
  if (record == null) return null;

  if (record['v'] != 1) return null;

  final clientId = record['client_id'];
  if (clientId is! String || clientId.isEmpty || clientId.runes.length > 64) {
    return null;
  }

  final contexts = asStringObjectMap(record['contexts']);
  if (contexts == null || contexts.length > _maxContexts) {
    return null;
  }

  return ReadStateBlob(
    clientId: clientId,
    contexts: sanitizeReadStateContexts(contexts),
  );
}

Map<String, int> sanitizeReadStateContexts(Map<String, Object?> contexts) {
  final sanitized = <String, int>{};
  for (final entry in contexts.entries) {
    if (utf8.encode(entry.key).length > 256) continue;

    final value = entry.value;
    if (value is! int) continue;
    if (value < 0 || value > 4294967295) continue;

    sanitized[entry.key] = value;
  }
  return sanitized;
}

DecodedReadStateEvent? decodeReadStateEvent(
  NostrEvent event, {
  required String pubkey,
  required ReadStateDecrypt decrypt,
}) {
  if (event.pubkey.toLowerCase() != pubkey.toLowerCase()) {
    return null;
  }
  if (!hasValidReadStateTags(event)) {
    return null;
  }

  final dTag = event.tags.firstWhere(
    (tag) => tag.isNotEmpty && tag[0] == 'd',
  )[1];

  final String plaintext;
  try {
    plaintext = decrypt(event.content);
  } catch (e) {
    debugPrint(
      '[ReadStateManager] decrypt failed for event ${event.id.substring(0, 8)}…: $e',
    );
    return null;
  }

  final blob = decodeReadStateBlob(plaintext);
  if (blob == null) {
    debugPrint(
      '[ReadStateManager] blob decode failed for event ${event.id.substring(0, 8)}…',
    );
    return null;
  }

  return DecodedReadStateEvent(event: event, dTag: dTag, blob: blob);
}

Map<String, int> mergeReadStateContexts(
  Iterable<Map<String, int>> contextSets,
) {
  final merged = <String, int>{};
  for (final contexts in contextSets) {
    for (final entry in contexts.entries) {
      final current = merged[entry.key] ?? 0;
      if (entry.value > current) {
        merged[entry.key] = entry.value;
      }
    }
  }
  return merged;
}
