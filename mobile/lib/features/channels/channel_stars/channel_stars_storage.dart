import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

String channelStarsKey(String pubkey) => 'buzz.channel-stars.v1:$pubkey';

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

  ChannelStarStore read(String pubkey) {
    final raw = _prefs.getString(channelStarsKey(pubkey));
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

  void write(String pubkey, ChannelStarStore store) {
    _prefs.setString(channelStarsKey(pubkey), jsonEncode(store.toJson()));
  }
}
