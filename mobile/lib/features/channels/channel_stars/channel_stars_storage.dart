import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

import '../../../shared/theme/community_theme_preference.dart';

/// Channel ids belong to one community, so the cache is keyed by relay as
/// well as identity. One identity can be paired with several communities.
String channelStarsKey(String pubkey, String relayUrl) =>
    'buzz.channel-stars.v1:$pubkey:${Uri.encodeComponent(normalizeCommunityRelayUrl(relayUrl))}';

/// The earlier key, shared by every community of the identity.
String legacyChannelStarsKey(String pubkey) => 'buzz.channel-stars.v1:$pubkey';

class ChannelStarEntry {
  final bool starred;
  final int updatedAt;

  const ChannelStarEntry({required this.starred, required this.updatedAt});

  Map<String, dynamic> toJson() => {'starred': starred, 'updatedAt': updatedAt};

  factory ChannelStarEntry.fromJson(Map<String, dynamic> json) =>
      ChannelStarEntry(
        starred: json['starred'] as bool,
        updatedAt: json['updatedAt'] as int,
      );
}

class ChannelStarStore {
  final int version;
  final Map<String, ChannelStarEntry> channels;

  const ChannelStarStore({this.version = 1, this.channels = const {}});

  Map<String, dynamic> toJson() => {
    'version': version,
    'channels': {for (final e in channels.entries) e.key: e.value.toJson()},
  };

  factory ChannelStarStore.fromJson(Map<String, dynamic> json) {
    final rawChannels = json['channels'];
    final channels = <String, ChannelStarEntry>{};
    if (rawChannels is Map) {
      for (final entry in rawChannels.entries) {
        if (entry.key is String && entry.value is Map<String, dynamic>) {
          final v = entry.value as Map<String, dynamic>;
          if (v['starred'] is bool && v['updatedAt'] is int) {
            channels[entry.key as String] = ChannelStarEntry.fromJson(v);
          }
        }
      }
    }
    return ChannelStarStore(version: 1, channels: channels);
  }
}

ChannelStarStore mergeStores(ChannelStarStore local, ChannelStarStore remote) {
  // Per-channel max-updatedAt merge:
  // For each channel ID in the union, keep the entry with the highest updatedAt.
  final merged = <String, ChannelStarEntry>{...local.channels};
  for (final entry in remote.channels.entries) {
    final existing = merged[entry.key];
    if (existing == null || entry.value.updatedAt > existing.updatedAt) {
      merged[entry.key] = entry.value;
    }
  }
  return ChannelStarStore(channels: merged);
}

class ChannelStarsStorage {
  final SharedPreferences _prefs;

  ChannelStarsStorage(this._prefs);

  ChannelStarStore read(String pubkey, String relayUrl) {
    final key = channelStarsKey(pubkey, relayUrl);
    // The first community to open claims the shared entry, which may hold
    // changes not yet published. Its channel ids only match that community's
    // channels, and the relay copy it merges with is already shared the same way.
    final legacyKey = legacyChannelStarsKey(pubkey);
    final legacy = _prefs.getString(legacyKey);
    if (legacy != null) {
      _prefs.remove(legacyKey);
      if (!_prefs.containsKey(key)) _prefs.setString(key, legacy);
    }

    final raw = _prefs.getString(key);
    if (raw == null || raw.isEmpty) {
      return const ChannelStarStore();
    }

    try {
      final parsed = jsonDecode(raw);
      if (parsed is! Map<String, dynamic>) {
        return const ChannelStarStore();
      }
      if (parsed['version'] != 1) {
        return const ChannelStarStore();
      }
      return ChannelStarStore.fromJson(parsed);
    } catch (_) {
      return const ChannelStarStore();
    }
  }

  Future<bool> write(String pubkey, String relayUrl, ChannelStarStore store) =>
      _prefs.setString(
        channelStarsKey(pubkey, relayUrl),
        jsonEncode(store.toJson()),
      );
}
