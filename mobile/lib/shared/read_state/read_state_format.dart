import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';

import '../relay/nostr_models.dart';

const readStateDTagPrefix = 'read-state:';
const readStateFetchLimit = 500;
const readStateHorizonSeconds = 30 * 24 * 60 * 60;
const _maxContexts = 10000;
const msgContextPrefix = 'msg:';
const threadContextPrefix = 'thread:';

String msgContextKey(String messageId) => '$msgContextPrefix$messageId';
String threadContextKey(String rootId) => '$threadContextPrefix$rootId';

/// Legacy catch-up marks. Catch-up now advances the channel or `thread:`
/// frontier, so these are only read, never written: older installations may
/// still write them. `activity:<channel>` reads ordinary top-level messages
/// up to its time; `thread-activity:<root>` reads that thread's replies.
/// Neither reads mentions, broadcasts or DMs in the channel.
String activityContextKey(String channelId) => 'activity:$channelId';
String threadActivityContextKey(String rootId) => 'thread-activity:$rootId';

/// Whether `activity:<channel>` reads a message: an ordinary top-level
/// message, not a reply, mention, broadcast or DM.
bool readByChannelCatchUp({
  required bool isDm,
  required bool isReply,
  required bool highPriority,
}) => !isDm && !isReply && !highPriority;

/// The plaintext budget for one published read-state slot. The web app
/// (`buzz-app`) uses the same limit, well under NIP-44's 64 KiB maximum.
const readStatePlaintextBytes = 40 * 1024;

/// The implicit read frontier: timestamps at or below it are read even
/// when no device has retained an explicit mark.
int readStateFloor(int nowUnixSeconds) =>
    (nowUnixSeconds - readStateHorizonSeconds).clamp(0, 4294967295);

/// Retain recent hints without count-based eviction. Override registers are
/// permanent; their co-located frontiers are retained by the owner as well.
Map<String, int> pruneStaleContexts(
  Map<String, int> contexts, {
  required int nowUnixSeconds,
}) => {
  for (final entry in contexts.entries)
    if (isOverrideContext(entry.key) ||
        entry.value >= readStateFloor(nowUnixSeconds))
      entry.key: entry.value,
};

/// Whether [contextId] is a manual-unread override counter (`ov_s:`,
/// `ov_c:`, `ov_b:`) or an escaped frontier (`esc:`). Counters are handled
/// as whole groups, never as frontiers.
bool isOverrideContext(String contextId) =>
    contextId.startsWith('ov_') || contextId.startsWith('esc:');

const _overridePrefixes = ['ov_s:', 'ov_c:', 'ov_b:'];

/// The raw context ID of an override counter key, or null for other keys.
String? _overrideTarget(String key) {
  for (final prefix in _overridePrefixes) {
    if (key.startsWith(prefix)) return key.substring(prefix.length);
  }
  return null;
}

/// The frontier key of raw context [contextId] on the wire. NIP-RS escapes
/// a raw ID that begins with `ov_` or `esc:` by prepending `esc:`.
String overrideFrontierKey(String contextId) =>
    contextId.startsWith('ov_') || contextId.startsWith('esc:')
    ? 'esc:$contextId'
    : contextId;

/// Whether [keys] form a legal override group for [target]: exactly
/// `ov_s:`, `ov_c:` and `ov_b:`, or `ov_c:` alone as a tombstone.
bool _isOverrideShape(String target, Iterable<String> keys) {
  final set = keys.toSet();
  return set.length == _overridePrefixes.length ||
      (set.length == 1 && set.contains('ov_c:$target'));
}

/// The override groups in [contexts] that have a legal shape, by target.
Map<String, Map<String, int>> _overrideGroups(Map<String, int> contexts) {
  final groups = <String, Map<String, int>>{};
  for (final entry in contexts.entries) {
    if (_overrideTarget(entry.key) case final target?) {
      (groups[target] ??= {})[entry.key] = entry.value;
    }
  }
  return {
    for (final MapEntry(key: target, value: group) in groups.entries)
      if (_isOverrideShape(target, group.keys)) target: group,
  };
}

/// Whether an override group for [target] is synchronized state. Only
/// whole-channel groups are: a `msg:` or `thread:` group, or any other
/// prefixed context, is ignored on read and never published.
bool isChannelOverrideTarget(String target) => !target.contains(':');

/// One merged manual-unread register: set counter, clear counter, baseline.
typedef OverrideRegister = ({int s, int c, int b});

/// The channel override registers carried by [contexts], from groups that
/// are complete and travel with their frontier. A tombstone reads as
/// `(0, C, 0)`.
Map<String, OverrideRegister> channelOverrideRegisters(
  Map<String, int> contexts,
) {
  final groups = completeOverrideGroups(contexts);
  return {
    for (final target in _overrideGroups(groups).keys)
      if (isChannelOverrideTarget(target))
        target: (
          s: groups['ov_s:$target'] ?? 0,
          c: groups['ov_c:$target'] ?? 0,
          b: groups['ov_b:$target'] ?? 0,
        ),
  };
}

/// NIP-RS's register merge: componentwise `max()`.
OverrideRegister mergeOverrideRegisters(
  OverrideRegister a,
  OverrideRegister b,
) => (s: max(a.s, b.s), c: max(a.c, b.c), b: max(a.b, b.b));

/// NIP-RS's Liveness Predicate against the merged frontier [frontier].
bool isOverrideActive(OverrideRegister register, int frontier) =>
    register.s > 0 && frontier <= register.b && register.s > register.c;

/// The canonical wire keys of [registers]: a live group publishes all three
/// counters, a dead one only its tombstone floor `ov_c: = max(S, C)`.
Map<String, int> canonicalOverrideEntries(
  Map<String, OverrideRegister> registers,
  int Function(String target) frontierOf,
) => {
  for (final MapEntry(key: target, value: r) in registers.entries)
    if (isOverrideActive(r, frontierOf(target))) ...{
      'ov_s:$target': r.s,
      'ov_c:$target': r.c,
      'ov_b:$target': r.b,
    } else
      'ov_c:$target': max(r.s, r.c),
};

/// The override keys of [contexts] that this app may carry. NIP-RS accepts
/// an override group only whole (see [_isOverrideShape]), and a group must
/// travel with its frontier, found in [contexts] or [frontiers]. Any other
/// group is rejected whole, so none of its keys is carried. Escaped
/// frontiers (`esc:`) are frontier keys, not counters, so each is carried
/// as it is.
Map<String, int> completeOverrideGroups(
  Map<String, int> contexts, {
  Map<String, int> frontiers = const {},
}) {
  final kept = <String, int>{
    for (final entry in contexts.entries)
      if (entry.key.startsWith('esc:')) entry.key: entry.value,
  };
  for (final MapEntry(key: target, value: group) in _overrideGroups(
    contexts,
  ).entries) {
    final frontier = overrideFrontierKey(target);
    if (contexts.containsKey(frontier) || frontiers.containsKey(frontier)) {
      kept.addAll(group);
    }
  }
  return kept;
}

int? maxReadAt(Iterable<int?> markers) {
  int? latest;
  for (final marker in markers) {
    if (marker == null) continue;
    if (latest == null || marker > latest) {
      latest = marker;
    }
  }
  return latest;
}

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

/// The valid entries of a decoded `contexts` object. Override counters are
/// checked as whole groups first, on the raw values, as NIP-RS requires: one
/// invalid sibling rejects its whole group, so dropping it alone can never
/// turn a live group into a tombstone. Other `ov_` keys are reserved and
/// dropped. Every other entry is checked on its own.
Map<String, int> sanitizeReadStateContexts(Map<String, Object?> contexts) {
  bool valid(String key, Object? value) =>
      utf8.encode(key).length <= 256 &&
      value is int &&
      value >= 0 &&
      value <= 4294967295;
  final groups = <String, Map<String, Object?>>{};
  for (final entry in contexts.entries) {
    if (_overrideTarget(entry.key) case final target?) {
      (groups[target] ??= {})[entry.key] = entry.value;
    }
  }
  final rejected = {
    for (final MapEntry(key: target, value: group) in groups.entries)
      if (!_isOverrideShape(target, group.keys) ||
          group.entries.any((entry) => !valid(entry.key, entry.value)))
        target,
  };
  final sanitized = <String, int>{};
  for (final entry in contexts.entries) {
    if (entry.key.startsWith('ov_')) {
      final target = _overrideTarget(entry.key);
      if (target == null || rejected.contains(target)) continue;
    }
    final value = entry.value;
    if (valid(entry.key, value)) sanitized[entry.key] = value as int;
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
