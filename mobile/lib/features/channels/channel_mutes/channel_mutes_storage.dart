import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

String channelMutesKey(String pubkey) => 'buzz.channel-mutes.v1:$pubkey';

class ChannelMuteEntry {
  final bool muted;
  final int updatedAt;

  const ChannelMuteEntry({required this.muted, required this.updatedAt});

  Map<String, dynamic> toJson() => {'muted': muted, 'updatedAt': updatedAt};

  factory ChannelMuteEntry.fromJson(Map<String, dynamic> json) =>
      ChannelMuteEntry(
        muted: json['muted'] as bool,
        updatedAt: json['updatedAt'] as int,
      );
}

class ChannelMuteStore {
  final int version;
  final Map<String, ChannelMuteEntry> channels;

  const ChannelMuteStore({this.version = 1, this.channels = const {}});

  Map<String, dynamic> toJson() => {
    'version': version,
    'channels': {for (final e in channels.entries) e.key: e.value.toJson()},
  };

  factory ChannelMuteStore.fromJson(Map<String, dynamic> json) {
    final rawChannels = json['channels'];
    final channels = <String, ChannelMuteEntry>{};
    if (rawChannels is Map) {
      for (final entry in rawChannels.entries) {
        if (entry.key is String && entry.value is Map<String, dynamic>) {
          final v = entry.value as Map<String, dynamic>;
          if (v['muted'] is bool && v['updatedAt'] is int) {
            channels[entry.key as String] = ChannelMuteEntry.fromJson(v);
          }
        }
      }
    }
    return ChannelMuteStore(version: 1, channels: channels);
  }
}

ChannelMuteStore mergeStores(ChannelMuteStore local, ChannelMuteStore remote) {
  // Per-channel max-updatedAt merge:
  // For each channel ID in the union, keep the entry with the highest updatedAt.
  final merged = <String, ChannelMuteEntry>{...local.channels};
  for (final entry in remote.channels.entries) {
    final existing = merged[entry.key];
    if (existing == null || entry.value.updatedAt > existing.updatedAt) {
      merged[entry.key] = entry.value;
    }
  }
  return ChannelMuteStore(channels: merged);
}

class ChannelMutesStorage {
  final SharedPreferences _prefs;

  ChannelMutesStorage(this._prefs);

  ChannelMuteStore read(String pubkey) {
    final raw = _prefs.getString(channelMutesKey(pubkey));
    if (raw == null || raw.isEmpty) {
      return const ChannelMuteStore();
    }

    try {
      final parsed = jsonDecode(raw);
      if (parsed is! Map<String, dynamic>) {
        return const ChannelMuteStore();
      }
      if (parsed['version'] != 1) {
        return const ChannelMuteStore();
      }
      return ChannelMuteStore.fromJson(parsed);
    } catch (_) {
      return const ChannelMuteStore();
    }
  }

  void write(String pubkey, ChannelMuteStore store) {
    _prefs.setString(channelMutesKey(pubkey), jsonEncode(store.toJson()));
  }
}
