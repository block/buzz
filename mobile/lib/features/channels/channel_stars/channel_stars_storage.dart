import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

import '../../../shared/read_state/sidebar_flags.dart';
import '../../../shared/theme/community_theme_preference.dart';

/// Channel ids belong to one community, so the cache is keyed by relay as
/// well as identity. One identity can be paired with several communities.
enum ChannelFlagKind {
  stars('channel-stars'),
  pins('channel-pins');

  const ChannelFlagKind(this.coordinate);
  final String coordinate;
}

String channelStarsKey(
  String pubkey,
  String relayUrl, [
  ChannelFlagKind kind = ChannelFlagKind.stars,
]) =>
    'buzz.${kind.coordinate}.v1:$pubkey:${Uri.encodeComponent(normalizeCommunityRelayUrl(relayUrl))}';

/// The earlier key, shared by every community of the identity.
String legacyChannelStarsKey(String pubkey) => 'buzz.channel-stars.v1:$pubkey';

class ChannelStarEntry implements SidebarFlagEntry {
  final bool starred;

  @override
  bool get enabled => starred;

  @override
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

  factory ChannelStarStore.fromJson(
    Map<String, dynamic> json, {
    bool strict = false,
  }) {
    final rawChannels = json['channels'];
    if (strict && (json['version'] != 1 || rawChannels is! Map)) {
      throw const FormatException('Invalid channel stars record');
    }
    final channels = <String, ChannelStarEntry>{};
    if (rawChannels is Map) {
      for (final entry in rawChannels.entries) {
        if (entry.key is String && entry.value is Map<String, dynamic>) {
          final v = entry.value as Map<String, dynamic>;
          final timestamp = v['updatedAt'];
          if (v['starred'] is bool && timestamp is int && timestamp >= 0) {
            channels[entry.key as String] = ChannelStarEntry.fromJson(v);
            continue;
          }
        }
        if (strict) throw const FormatException('Invalid channel star entry');
      }
    }
    return ChannelStarStore(version: 1, channels: channels);
  }
}

class ChannelStarsStorage {
  final SharedPreferences _prefs;

  ChannelStarsStorage(this._prefs);

  ChannelStarStore read(
    String pubkey,
    String relayUrl, [
    ChannelFlagKind kind = ChannelFlagKind.stars,
  ]) {
    // The shared entry may hold another community's channels, and the relay
    // copy is authoritative, so drop it rather than let one community claim it.
    if (kind == ChannelFlagKind.stars) {
      _prefs.remove(legacyChannelStarsKey(pubkey));
    }
    final raw = _prefs.getString(channelStarsKey(pubkey, relayUrl, kind));
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
}
