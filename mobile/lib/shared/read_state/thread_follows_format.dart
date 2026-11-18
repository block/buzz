import 'dart:convert';

import '../relay/nostr_models.dart';
import 'read_state_format.dart';

/// Synced thread follow choices (NIP-RS, Buzz Thread Follow Choices): each
/// installation publishes its own explicit follows and unfollows in its own
/// `thread-follows:<slot>` coordinates, and readers merge every slot.
const threadFollowsDTagPrefix = 'thread-follows:';
const threadFollowsTag = 'thread-follows';

/// At most this many own follow coordinates per installation.
const threadFollowsMaxSlots = 16;

final _slotId = RegExp(r'^[0-9a-f]{32}$');
final _threadKey = RegExp(r'^[^:]+:[0-9a-f]{64}$');

/// The wire key of a thread: `<channel-id>:<root-event-id>`.
String threadFollowKey(String channelId, String rootId) => '$channelId:$rootId';

/// The root event id of a [threadFollowKey].
String threadFollowRoot(String key) => key.substring(key.lastIndexOf(':') + 1);

/// The channel id of a [threadFollowKey].
String threadFollowChannel(String key) =>
    key.substring(0, key.lastIndexOf(':'));

bool isValidThreadFollowKey(String key) => _threadKey.hasMatch(key);

/// One explicit choice: follow or unfollow, at the time of the action.
typedef ThreadFollowChoice = ({bool follow, int at});

/// The merge rule: the greater `at` wins; on equal `at`, unfollow wins.
ThreadFollowChoice mergeThreadFollowChoices(
  ThreadFollowChoice a,
  ThreadFollowChoice b,
) {
  if (a.at != b.at) return a.at > b.at ? a : b;
  return a.follow ? b : a;
}

/// The slot id of a follow coordinate's `d` tag, or null when the event is
/// not a valid follow coordinate (and so must be ignored).
String? threadFollowsSlotOf(NostrEvent event) {
  final dTags = [
    for (final tag in event.tags)
      if (tag.isNotEmpty && tag[0] == 'd') tag,
  ];
  if (dTags.length != 1 || dTags.single.length < 2) return null;
  final d = dTags.single[1];
  if (!d.startsWith(threadFollowsDTagPrefix)) return null;
  final slot = d.substring(threadFollowsDTagPrefix.length);
  return _slotId.hasMatch(slot) ? slot : null;
}

/// One decoded follow coordinate. [entries] keeps each entry's raw object so
/// its author can carry unknown fields unchanged; [extra] does the same for
/// unknown top-level fields.
class ThreadFollowsBlob {
  final String clientId;
  final Map<String, Map<String, Object?>> entries;
  final Map<String, Object?> extra;

  const ThreadFollowsBlob({
    required this.clientId,
    required this.entries,
    this.extra = const {},
  });

  Map<String, ThreadFollowChoice> get choices => {
    for (final MapEntry(:key, :value) in entries.entries)
      key: (follow: value['follow'] as bool, at: value['at'] as int),
  };

  /// Deterministic plaintext: sorted thread keys, so an unchanged slot
  /// encodes to the same bytes.
  String encode() => jsonEncode({
    ...extra,
    'v': 1,
    'client_id': clientId,
    'threads': {
      for (final key in entries.keys.toList()..sort()) key: entries[key],
    },
  });
}

/// Decodes [plaintext], or returns null when the coordinate is unreadable:
/// its entries are then unknown, not absent.
ThreadFollowsBlob? decodeThreadFollowsBlob(String plaintext) {
  final Object? parsed;
  try {
    parsed = jsonDecode(plaintext);
  } catch (_) {
    return null;
  }
  final record = asStringObjectMap(parsed);
  if (record == null || record['v'] != 1) return null;
  final clientId = record['client_id'];
  if (clientId is! String) return null;
  final threads = asStringObjectMap(record['threads']);
  if (threads == null) return null;
  final entries = <String, Map<String, Object?>>{};
  for (final MapEntry(:key, :value) in threads.entries) {
    final entry = asStringObjectMap(value);
    final at = entry?['at'];
    if (!isValidThreadFollowKey(key) ||
        entry == null ||
        entry['follow'] is! bool ||
        at is! int ||
        at < 0 ||
        at > 4294967295) {
      return null;
    }
    entries[key] = Map.unmodifiable(entry);
  }
  return ThreadFollowsBlob(
    clientId: clientId,
    entries: entries,
    extra: {
      for (final MapEntry(:key, :value) in record.entries)
        if (key != 'v' && key != 'client_id' && key != 'threads') key: value,
    },
  );
}

/// Places [keys] into follow slots of at most [maxBytes] plaintext each,
/// keeping every key in the slot it already occupies in [previous] and
/// filling the rest first-fit. Throws [StateError] past
/// [threadFollowsMaxSlots] slots.
List<Set<String>> packThreadFollowKeys(
  Map<String, Map<String, Object?>> entries, {
  required String clientId,
  List<Set<String>> previous = const [],
  List<Map<String, Object?>> extras = const [],
  int maxBytes = readStatePlaintextBytes,
}) {
  // Each slot is measured with its own unknown fields; new slots have none.
  int size(Set<String> keys, int index) => utf8
      .encode(
        ThreadFollowsBlob(
          clientId: clientId,
          entries: {for (final key in keys) key: entries[key]!},
          extra: index < extras.length ? extras[index] : const {},
        ).encode(),
      )
      .length;
  int cost(String key) =>
      utf8.encode(jsonEncode(key)).length +
      1 +
      utf8.encode(jsonEncode(entries[key])).length;
  final slots = [
    for (final old in previous) {...old.where(entries.containsKey)},
  ];
  if (slots.isEmpty) slots.add({});
  // Exact compact JSON sizes, adding each entry and its comma without
  // re-encoding whole slots.
  final sizes = [
    for (var index = 0; index < slots.length; index++)
      size(slots[index], index),
  ];
  final assigned = {for (final slot in slots) ...slot};
  for (final key
      in entries.keys.where((k) => !assigned.contains(k)).toList()..sort()) {
    final bytes = cost(key);
    var placed = false;
    for (var index = 0; index < slots.length; index++) {
      final extra = bytes + (slots[index].isEmpty ? 0 : 1);
      if (sizes[index] + extra <= maxBytes) {
        slots[index].add(key);
        sizes[index] += extra;
        placed = true;
        break;
      }
    }
    if (!placed) {
      sizes.add(size({}, slots.length) + bytes);
      slots.add({key});
    }
  }
  if (slots.length > threadFollowsMaxSlots ||
      sizes.any((size) => size > maxBytes)) {
    throw StateError('Thread follow choices exceed their slot capacity');
  }
  return slots;
}
