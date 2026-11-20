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

/// Desktop sorts Solo by recent activity until the user picks otherwise.
ChannelSortMode defaultSortModeFor(String groupKey) =>
    groupKey == 'solo' ? ChannelSortMode.recent : kDefaultSortMode;

/// Desktop's unified sort coordinate. When its head exists desktop ignores
/// `channel-sort`, so mobile reads it and stops writing the legacy one.
const workspaceSortCoordinate = 'channel-workspace-sort';

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

  /// Reads a `channel-workspace-sort` payload. Register metadata, when
  /// present, is authoritative over the plain `groups` map; a null register
  /// clears that section's choice.
  factory ChannelSortStore.fromWorkspaceJson(Map<String, dynamic> json) {
    final meta = json['meta'];
    if (meta is! Map) return ChannelSortStore.fromJson(json);
    final registers = meta['g'];
    if (meta['v'] != 1 || (registers != null && registers is! Map)) {
      throw const FormatException('Unsupported sort metadata');
    }
    return ChannelSortStore(
      groups: {
        for (final MapEntry(:key, :value)
            in (registers as Map? ?? const {}).entries)
          if (key is String && value is List && value.length == 3)
            key: ?ChannelSortMode.fromWire(value[2]),
      },
    );
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
  // As desktop: newest activity first, counting at least from creation so a
  // chat just started leads before its first message lands.
  int recentMs(Channel channel) {
    final created = channel.createdAt.millisecondsSinceEpoch;
    final last = channel.lastMessageAt?.millisecondsSinceEpoch;
    return last == null || last < created ? created : last;
  }

  sorted.sort((left, right) {
    final recent = recentMs(right).compareTo(recentMs(left));
    return recent != 0 ? recent : compareChannelsByName(left, right);
  });
  return sorted;
}

class ChannelSortSyncState {
  final int updatedAt;
  final String eventId;
  final bool hasPendingLocalChanges;
  final int pendingUpdatedAt;

  const ChannelSortSyncState({
    this.updatedAt = 0,
    this.eventId = '',
    this.hasPendingLocalChanges = false,
    this.pendingUpdatedAt = 0,
  });

  Map<String, dynamic> toJson() => {
    'updatedAt': updatedAt,
    'eventId': eventId,
    'hasPendingLocalChanges': hasPendingLocalChanges,
    'pendingUpdatedAt': pendingUpdatedAt,
  };

  factory ChannelSortSyncState.fromJson(Map<String, dynamic> json) {
    final updatedAt = json['updatedAt'] is int ? json['updatedAt'] as int : 0;
    final hasPendingLocalChanges = json['hasPendingLocalChanges'] == true;
    final storedPendingUpdatedAt = json['pendingUpdatedAt'];
    // Older builds used updatedAt for both the remote cursor and local edit
    // stamp. Preserve the pending guard while resetting that ambiguous cursor.
    final isLegacyPending =
        hasPendingLocalChanges && storedPendingUpdatedAt is! int;
    return ChannelSortSyncState(
      updatedAt: isLegacyPending ? 0 : updatedAt,
      eventId: isLegacyPending
          ? ''
          : (json['eventId'] is String ? json['eventId'] as String : ''),
      hasPendingLocalChanges: hasPendingLocalChanges,
      pendingUpdatedAt: storedPendingUpdatedAt is int
          ? storedPendingUpdatedAt
          : (hasPendingLocalChanges ? updatedAt : 0),
    );
  }
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

  ChannelSortSyncState readSyncState(String pubkey, String relayUrl) {
    final raw = _prefs.getString(_syncStateKey(pubkey, relayUrl));
    if (raw == null || raw.isEmpty) return const ChannelSortSyncState();
    try {
      final parsed = jsonDecode(raw);
      return parsed is Map<String, dynamic>
          ? ChannelSortSyncState.fromJson(parsed)
          : const ChannelSortSyncState();
    } catch (_) {
      return const ChannelSortSyncState();
    }
  }

  void writeSyncState(
    String pubkey,
    String relayUrl,
    ChannelSortSyncState state,
  ) {
    _prefs.setString(
      _syncStateKey(pubkey, relayUrl),
      jsonEncode(state.toJson()),
    );
  }

  /// The last unified sort head, so a cold start sorts as desktop does.
  ({ChannelSortStore store, int createdAt, String eventId})? readWorkspace(
    String pubkey,
    String relayUrl,
  ) {
    final raw = _prefs.getString(_workspaceKey(pubkey, relayUrl));
    if (raw == null) return null;
    try {
      if (jsonDecode(raw) case {
        'store': final Map<String, dynamic> store,
        'createdAt': final int createdAt,
        'eventId': final String eventId,
      }) {
        return (
          store: ChannelSortStore.fromJson(store),
          createdAt: createdAt,
          eventId: eventId,
        );
      }
    } on FormatException {
      // The relay head replaces an unreadable cache.
    }
    return null;
  }

  void writeWorkspace(
    String pubkey,
    String relayUrl,
    ChannelSortStore store,
    int createdAt,
    String eventId,
  ) {
    _prefs.setString(
      _workspaceKey(pubkey, relayUrl),
      jsonEncode({
        'store': store.toJson(),
        'createdAt': createdAt,
        'eventId': eventId,
      }),
    );
  }

  String _workspaceKey(String pubkey, String relayUrl) =>
      '${channelSortKey(pubkey, relayUrl)}:workspace';

  String _syncStateKey(String pubkey, String relayUrl) =>
      '${channelSortKey(pubkey, relayUrl)}:sync';
}
