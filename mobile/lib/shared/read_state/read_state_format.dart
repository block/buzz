import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../relay/nostr_models.dart';

const readStateDTagPrefix = 'read-state:';
const readStateFetchLimit = 500;
const readStateHorizonSeconds = 7 * 24 * 60 * 60;
const _maxContexts = 10000;
const msgContextPrefix = 'msg:';
const threadContextPrefix = 'thread:';

String msgContextKey(String messageId) => '$msgContextPrefix$messageId';
String threadContextKey(String rootId) => '$threadContextPrefix$rootId';

/// Catch-up marks, written by the web and desktop app (`buzz-app`) when the
/// reader reaches the bottom of a channel or thread. `activity:<channel>`
/// reads ordinary top-level messages up to its time; `thread-activity:<root>`
/// reads that thread's replies. Neither reads mentions, broadcasts or DMs
/// in the channel, so neither is a channel marker.
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

/// The most `msg:` and `thread:` marks this device saves. The desktop app
/// uses the same limit.
const localMaxPrunableContexts = 1000;

bool _isPrunableContext(String contextId) =>
    contextId.startsWith(msgContextPrefix) ||
    contextId.startsWith(threadContextPrefix);

/// The marks from [contexts] that this device saves, following the desktop
/// app's `pruneStaleContexts`. It drops `msg:` and `thread:` marks older
/// than the fetch horizon, then keeps the newest [localMaxPrunableContexts]
/// of them. Each automatic read writes a `msg:` mark, so without this bound
/// the saved state, and the time to save it, would grow with every read.
/// Channel and catch-up marks are kept: there is one per channel or thread,
/// and losing one would show read messages as unread again.
Map<String, int> pruneStaleContexts(
  Map<String, int> contexts, {
  required int nowUnixSeconds,
}) {
  final cutoff = nowUnixSeconds - readStateHorizonSeconds;
  final kept = <String, int>{};
  final prunable = <MapEntry<String, int>>[];
  for (final entry in contexts.entries) {
    if (!_isPrunableContext(entry.key)) {
      kept[entry.key] = entry.value;
    } else if (entry.value >= cutoff) {
      prunable.add(entry);
    }
  }
  prunable.sort((a, b) {
    final byTime = b.value.compareTo(a.value);
    return byTime != 0 ? byTime : a.key.compareTo(b.key);
  });
  for (final entry in prunable.take(localMaxPrunableContexts)) {
    kept[entry.key] = entry.value;
  }
  return kept;
}

/// Whether [contextId] belongs to the web app's manual-unread overrides
/// (`ov_s:`, `ov_c:`, `ov_b:`) or their escaped frontiers (`esc:`). This app
/// does not read overrides. It does not take them from other slots, but it
/// carries the ones already in its own slot unchanged: NIP-RS forbids
/// dropping them, and an old version of this app copied them there.
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

/// The frontier keys that must travel with the override groups in
/// [contexts] (NIP-RS's co-location rule).
Set<String> overrideGroupFrontierKeys(Map<String, int> contexts) => {
  for (final target in _overrideGroups(contexts).keys)
    overrideFrontierKey(target),
};

/// Whether this device republishes a mark it merged from another device's
/// slot. Message marks are not republished: each covers one message, and the
/// web app prunes them once a catch-up mark reads the message. Broad marks
/// are republished so they outlive the fetch horizon of the slot that wrote
/// them.
bool republishesMergedContext(String contextId) =>
    !contextId.startsWith(msgContextPrefix) && !isOverrideContext(contextId);

/// Keep order for a slot that is over budget, following `buzz-app`'s
/// retention: channel marks first, then thread marks, then catch-up marks,
/// then message marks.
int _retentionScope(String key) {
  if (!key.contains(':')) return 0;
  if (key.startsWith(threadContextPrefix)) return 1;
  if (key.startsWith('activity:') || key.startsWith('thread-activity:')) {
    return 2;
  }
  return 3;
}

/// The share of the budget that broad marks may fill before recent use
/// decides. The rest always goes to the most recently written marks, so a
/// new read is never left out because old channel or thread marks fill the
/// budget. `buzz-app` uses the same share.
const _scopedShare = 0.75;

/// The marks from [contexts] that fit one read-state slot for [clientId]
/// within [maxBytes] of plaintext, following `buzz-app`'s retention.
///
/// Up to three quarters of the budget goes to broad marks first: channel
/// marks, then thread marks, then catch-up marks, then message marks. The
/// rest goes by [recent], the time each mark was last written, newest first,
/// so reading old history still syncs. A slot over NIP-44's limit cannot be
/// encrypted, so without this cap sync would stop.
///
/// [carried] override groups are kept whole, ahead of every mark, together
/// with each group's frontier from [contexts]: NIP-RS requires a context's
/// frontier and its override keys in the same event. Incomplete groups are
/// left out (see [completeOverrideGroups]). If the groups do not fit, this
/// returns null, and the caller must leave the slot as it is instead of
/// publishing part of a group.
Map<String, int>? retainReadStateContexts(
  Map<String, int> contexts, {
  required String clientId,
  Map<String, int> recent = const {},
  Map<String, int> carried = const {},
  int maxBytes = readStatePlaintextBytes,
}) {
  // JSON-encoded bytes, so escaped characters are counted as published.
  int bytesOf(Object? value) => utf8.encode(jsonEncode(value)).length;
  final groups = completeOverrideGroups(carried, frontiers: contexts);
  final reserved = <String, int>{...groups};
  for (final key in overrideGroupFrontierKeys(groups)) {
    final frontier = contexts[key];
    if (frontier != null && frontier > (reserved[key] ?? -1)) {
      reserved[key] = frontier;
    }
  }
  final fixedBytes = bytesOf(
    ReadStateBlob(clientId: clientId, contexts: reserved).toJson(),
  );
  if (fixedBytes > maxBytes || reserved.length > _maxContexts) return null;
  // Marks share what the carried keys leave.
  final markBytes = maxBytes - fixedBytes;
  final markCount = _maxContexts - reserved.length;
  var used = 0;
  var count = 0;
  bool take(MapEntry<String, int> entry, double share) {
    // A key, its colon, its value and one separating comma.
    final cost = bytesOf(entry.key) + 2 + '${entry.value}'.length;
    if (used + cost > markBytes * share || count >= markCount * share) {
      return false;
    }
    used += cost;
    count++;
    return true;
  }

  int byUse(MapEntry<String, int> a, MapEntry<String, int> b) {
    final byRecent = (recent[b.key] ?? 0).compareTo(recent[a.key] ?? 0);
    if (byRecent != 0) return byRecent;
    final byTime = b.value.compareTo(a.value);
    return byTime != 0 ? byTime : a.key.compareTo(b.key);
  }

  final retained = <String, int>{...reserved};
  final scoped =
      contexts.entries
          .where((entry) => !reserved.containsKey(entry.key))
          .toList()
        ..sort((a, b) {
          final byScope = _retentionScope(
            a.key,
          ).compareTo(_retentionScope(b.key));
          return byScope != 0 ? byScope : byUse(a, b);
        });
  for (final entry in scoped) {
    // Stop at the first broad mark that does not fit, so a narrower mark
    // never takes the share ahead of it.
    if (!take(entry, _scopedShare)) break;
    retained[entry.key] = entry.value;
  }
  final byRecent =
      contexts.entries
          .where((entry) => !reserved.containsKey(entry.key))
          .toList()
        ..sort(byUse);
  for (final entry in byRecent) {
    if (!retained.containsKey(entry.key) && take(entry, 1)) {
      retained[entry.key] = entry.value;
    }
  }
  return retained;
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
