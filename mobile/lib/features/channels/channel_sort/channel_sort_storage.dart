import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

import '../channel.dart';

String normalizeChannelSortRelayUrl(String relayUrl) =>
    relayUrl.trim().replaceFirst(RegExp(r'/+$'), '').toLowerCase();

String channelSortKey(String pubkey, String relayUrl) =>
    'buzz.channel-sort.v1:$pubkey:${Uri.encodeComponent(normalizeChannelSortRelayUrl(relayUrl))}';

String legacyChannelSortKey(String pubkey) => 'buzz.channel-sort.v1:$pubkey';

/// Per-group sidebar sort mode. The wire values match desktop exactly.
enum ChannelSortMode {
  alpha('alpha'),
  recent('recent');

  final String wireValue;

  const ChannelSortMode(this.wireValue);

  static ChannelSortMode? fromWire(Object? value) {
    if (value == 'alpha') return ChannelSortMode.alpha;
    if (value == 'recent') return ChannelSortMode.recent;
    return null;
  }
}

const ChannelSortMode kDefaultSortMode = ChannelSortMode.alpha;

String sectionSortGroupKey(String sectionId) => 'section:$sectionId';

class ChannelSortStore {
  final int version;
  final Map<String, ChannelSortMode> groups;

  const ChannelSortStore({this.version = 1, this.groups = const {}});

  Map<String, dynamic> toJson() => {
    'version': version,
    'groups': {
      for (final entry in groups.entries) entry.key: entry.value.wireValue,
    },
  };

  factory ChannelSortStore.fromJson(Map<String, dynamic> json) {
    final groups = <String, ChannelSortMode>{};
    final rawGroups = json['groups'];
    if (rawGroups is Map) {
      for (final entry in rawGroups.entries) {
        final mode = ChannelSortMode.fromWire(entry.value);
        if (entry.key is String && mode != null) {
          groups[entry.key as String] = mode;
        }
      }
    }
    return ChannelSortStore(groups: groups);
  }
}

ChannelSortStore stripOrphanedSectionModes(
  ChannelSortStore store,
  Iterable<String> liveSectionIds,
) {
  final liveKeys = {for (final id in liveSectionIds) sectionSortGroupKey(id)};
  final kept = <String, ChannelSortMode>{
    for (final entry in store.groups.entries)
      if (!entry.key.startsWith('section:') || liveKeys.contains(entry.key))
        entry.key: entry.value,
  };
  if (kept.length == store.groups.length) return store;
  return ChannelSortStore(groups: kept);
}

/// Mobile and desktop both use a case-insensitive deterministic ordering.
/// The id tie-break keeps equal folded names stable across clients.
int compareChannelsByName(Channel left, Channel right) {
  final name = left.name.toLowerCase().compareTo(right.name.toLowerCase());
  return name != 0 ? name : left.id.compareTo(right.id);
}

List<Channel> sortChannelsForList(
  List<Channel> channels,
  ChannelSortMode mode,
) {
  final sorted = channels.toList();
  if (mode == ChannelSortMode.alpha) {
    sorted.sort(compareChannelsByName);
    return sorted;
  }
  sorted.sort((left, right) {
    final leftMs = left.lastMessageAt?.millisecondsSinceEpoch;
    final rightMs = right.lastMessageAt?.millisecondsSinceEpoch;
    if (leftMs != null && rightMs != null && leftMs != rightMs) {
      return rightMs.compareTo(leftMs);
    }
    if (leftMs != null && rightMs == null) return -1;
    if (leftMs == null && rightMs != null) return 1;
    return compareChannelsByName(left, right);
  });
  return sorted;
}

class ChannelSortStorage {
  final SharedPreferences _prefs;

  ChannelSortStorage(this._prefs);

  ChannelSortStore read(String pubkey, String relayUrl) {
    final scopedKey = channelSortKey(pubkey, relayUrl);
    final scoped = _readKey(scopedKey);
    if (scoped != null) return scoped;

    // One-time read-through migration from #2829 development builds. The
    // first active relay claims the legacy value; removing it prevents the
    // same unscoped preferences from bleeding into later communities.
    final legacyKey = legacyChannelSortKey(pubkey);
    final legacy = _readKey(legacyKey);
    if (legacy != null) {
      write(pubkey, relayUrl, legacy);
      _prefs.remove(legacyKey);
      return legacy;
    }
    return const ChannelSortStore();
  }

  ChannelSortStore? _readKey(String key) {
    final raw = _prefs.getString(key);
    if (raw == null || raw.isEmpty) {
      return null;
    }
    try {
      final parsed = jsonDecode(raw);
      if (parsed is! Map<String, dynamic> || parsed['version'] != 1) {
        return null;
      }
      return ChannelSortStore.fromJson(parsed);
    } catch (_) {
      return null;
    }
  }

  void write(String pubkey, String relayUrl, ChannelSortStore store) {
    _prefs.setString(
      channelSortKey(pubkey, relayUrl),
      jsonEncode(store.toJson()),
    );
  }
}
