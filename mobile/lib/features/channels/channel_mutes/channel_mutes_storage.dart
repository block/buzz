import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

import '../../../shared/read_state/sidebar_flags.dart';
import '../../../shared/theme/community_theme_preference.dart';

/// Channel ids belong to one community, so the cache is keyed by relay as
/// well as identity. One identity can be paired with several communities.
String channelMutesKey(String pubkey, String relayUrl) =>
    'buzz.channel-mutes.v1:$pubkey:${Uri.encodeComponent(normalizeCommunityRelayUrl(relayUrl))}';

/// The earlier key, shared by every community of the identity.
String legacyChannelMutesKey(String pubkey) => 'buzz.channel-mutes.v1:$pubkey';

class ChannelMuteEntry implements SidebarFlagEntry {
  final bool muted;

  @override
  bool get enabled => muted;

  @override
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

  /// Converts legacy timestamps and bounds the outgoing record.
  ChannelMuteStore forPublish(Set<String> changed) {
    final bounded = {
      for (final entry in channels.entries)
        entry.key: ChannelMuteEntry(
          muted: entry.value.muted,
          updatedAt: sidebarFlagTimeMillis(entry.value.updatedAt),
        ),
    };
    boundSidebarFlags(bounded, changed);
    return ChannelMuteStore(channels: bounded);
  }

  factory ChannelMuteStore.fromJson(
    Map<String, dynamic> json, {
    bool strict = false,
  }) {
    final rawChannels = json['channels'];
    if (strict && (json['version'] != 1 || rawChannels is! Map)) {
      throw const FormatException('Invalid channel mutes record');
    }
    final channels = <String, ChannelMuteEntry>{};
    if (rawChannels is Map) {
      for (final entry in rawChannels.entries) {
        if (entry.key is String && entry.value is Map<String, dynamic>) {
          final v = entry.value as Map<String, dynamic>;
          final timestamp = v['updatedAt'];
          if (v['muted'] is bool && timestamp is int && timestamp >= 0) {
            channels[entry.key as String] = ChannelMuteEntry.fromJson(v);
            continue;
          }
        }
        if (strict) throw const FormatException('Invalid channel mute entry');
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
    if (existing == null ||
        sidebarFlagTimeMillis(entry.value.updatedAt) >
            sidebarFlagTimeMillis(existing.updatedAt)) {
      merged[entry.key] = entry.value;
    }
  }
  return ChannelMuteStore(channels: merged);
}

class ChannelMutesStorage {
  final SharedPreferences _prefs;

  ChannelMutesStorage(this._prefs);

  ChannelMuteStore read(String pubkey, String relayUrl) {
    // The shared entry may hold another community's channels, and the relay
    // copy is authoritative, so drop it rather than let one community claim it.
    _prefs.remove(legacyChannelMutesKey(pubkey));
    final raw = _prefs.getString(channelMutesKey(pubkey, relayUrl));
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

  Future<bool> write(String pubkey, String relayUrl, ChannelMuteStore store) =>
      _prefs.setString(
        channelMutesKey(pubkey, relayUrl),
        jsonEncode(store.toJson()),
      );
}
