import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';

import '../relay/nostr_models.dart';

const readStateDTagPrefix = 'read-state:';

/// The isolated version-2 record for whole-channel manual unread (NIP-RS,
/// Expiring unread state). Read positions stay in the `read-state:` record.
const readStateUnreadDTagPrefix = 'read-state-unread-v2:';
const readStateUnreadTag = 'read-state-unread-v2';

/// A mark stays live at most this long after its set stamp.
const overrideLifetimeSeconds = 30 * 24 * 60 * 60;

/// A dead group is kept this much longer than the lifetime before discard.
const overrideDiscardGuardSeconds = 600;

/// A group stamped further ahead than this is deferred, not merged.
const overrideFutureLimitSeconds = 660;

/// An action stamp may run at most this far ahead of the local clock.
const overrideAllocationAheadSeconds = 60;
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
/// message, not a reply, mention, broadcast or DM. Callers pass a message
/// with the broadcast tag as [isReply], with or without a parent:
/// [highPriority] means only a mention of the reader.
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

/// NIP-RS's Liveness Predicate against the merged frontier [frontier], at
/// expiry clock [now]: a mark expires [overrideLifetimeSeconds] after its set.
bool isOverrideActive(
  OverrideRegister register,
  int frontier, {
  required int now,
}) =>
    register.s > 0 &&
    frontier <= register.b &&
    register.s > register.c &&
    register.s >= now - overrideLifetimeSeconds;

/// The greatest action stamp of [register]: its canonical floor K once dead.
int overrideStamp(OverrideRegister register) => max(register.s, register.c);

/// Whether a received group is a replay too old to merge.
bool isOverrideExpired(OverrideRegister register, {required int now}) =>
    overrideStamp(register) < now - overrideLifetimeSeconds;

/// Whether a dead group, its ceiling and its paired frontier may be dropped.
bool isOverrideDiscardable(OverrideRegister register, {required int now}) =>
    overrideStamp(register) <
    now - overrideLifetimeSeconds - overrideDiscardGuardSeconds;

/// The canonical wire keys of [registers]: a live group publishes all three
/// counters, a dead one only its tombstone floor `ov_c: = max(S, C)`.
/// The stamp an action on [register] takes (NIP-RS Expiring unread state),
/// or null when it would pass uint32 or run more than 60 seconds ahead.
int? allocateOverrideStamp(OverrideRegister register, {required int now}) {
  final stamp = max(now, overrideStamp(register) + 1);
  return stamp > 4294967295 || stamp > now + overrideAllocationAheadSeconds
      ? null
      : stamp;
}

/// Whether a received group is too far ahead to merge yet.
bool isOverrideDeferred(OverrideRegister register, {required int now}) =>
    overrideStamp(register) > now + overrideFutureLimitSeconds;

Map<String, int> canonicalOverrideEntries(
  Map<String, OverrideRegister> registers,
  int Function(String target) frontierOf, {
  required int now,
}) => {
  for (final MapEntry(key: target, value: r) in registers.entries)
    if (isOverrideActive(r, frontierOf(target), now: now)) ...{
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

  /// 1 for read positions, 2 for the expiring unread record.
  final int version;

  ReadStateBlob({
    required this.clientId,
    required Map<String, int> contexts,
    this.version = 1,
  }) : contexts = Map.unmodifiable(contexts);

  Map<String, dynamic> toJson() => {
    'v': version,
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

bool isValidReadStateDTag(String? value, {bool unread = false}) {
  final prefix = unread ? readStateUnreadDTagPrefix : readStateDTagPrefix;
  if (value == null || !value.startsWith(prefix)) {
    return false;
  }

  final slotId = value.substring(prefix.length);
  // Version 2 admits only the 32 lowercase hex slots it publishes.
  if (unread) return RegExp(r'^[0-9a-f]{32}$').hasMatch(slotId);
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

/// Whether [event] carries the version-2 unread record's tags.
bool hasValidUnreadStateTags(NostrEvent event) =>
    hasValidReadStateTags(event, unread: true);

bool hasValidReadStateTags(NostrEvent event, {bool unread = false}) {
  final dTags = event.tags.where((tag) => tag.isNotEmpty && tag[0] == 'd');
  if (dTags.length != 1) {
    return false;
  }
  final dTag = dTags.single;
  if (dTag.length < 2 || !isValidReadStateDTag(dTag[1], unread: unread)) {
    return false;
  }

  final tag = unread ? readStateUnreadTag : 'read-state';
  final tTags = event.tags.where(
    (t) => t.length >= 2 && t[0] == 't' && t[1] == tag,
  );
  return tTags.length == 1;
}

ReadStateBlob? decodeReadStateBlob(String plaintext, {int version = 1}) {
  final Object? parsed;
  try {
    parsed = jsonDecode(plaintext);
  } catch (_) {
    return null;
  }

  final record = asStringObjectMap(parsed);
  if (record == null) return null;

  if (record['v'] != version) return null;

  final clientId = record['client_id'];
  if (clientId is! String || clientId.isEmpty || clientId.runes.length > 64) {
    return null;
  }

  final contexts = asStringObjectMap(record['contexts']);
  if (contexts == null || contexts.length > _maxContexts) {
    return null;
  }

  final sanitized = sanitizeReadStateContexts(contexts);
  return ReadStateBlob(
    clientId: clientId,
    version: version,
    // Version 1 override keys are never v2 intent (no import); version 2
    // carries only channel groups and their paired frontiers.
    contexts: version == 1
        ? {
            for (final entry in sanitized.entries)
              if (!entry.key.startsWith('ov_')) entry.key: entry.value,
          }
        : _unreadContexts(sanitized),
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

/// A version-2 record's complete channel groups with their paired frontiers.
Map<String, int> _unreadContexts(Map<String, int> contexts) {
  final groups = completeOverrideGroups(contexts);
  return {
    for (final MapEntry(key: target, value: group) in _overrideGroups(
      groups,
    ).entries)
      if (isChannelOverrideTarget(target)) ...{
        ...group,
        overrideFrontierKey(target): contexts[overrideFrontierKey(target)]!,
      },
  };
}

DecodedReadStateEvent? decodeReadStateEvent(
  NostrEvent event, {
  required String pubkey,
  required ReadStateDecrypt decrypt,
}) {
  if (event.pubkey.toLowerCase() != pubkey.toLowerCase()) {
    return null;
  }
  final unread = hasValidUnreadStateTags(event);
  if (!unread && !hasValidReadStateTags(event)) {
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

  final blob = decodeReadStateBlob(plaintext, version: unread ? 2 : 1);
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
