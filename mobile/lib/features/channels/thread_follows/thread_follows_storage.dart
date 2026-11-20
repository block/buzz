import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

import '../../../shared/theme/community_theme_preference.dart';

/// Pre-sync thread follows: per identity and community, `{rootId,
/// followedAt}` with no channel. Synced follows replaced this store; it is
/// kept only as the source of the one-time legacy import, which retires each
/// entry once its thread's channel is known and it is imported.
String threadFollowsKey(String pubkey, String relayUrl) =>
    'buzz-thread-follows.v1:$pubkey:${Uri.encodeComponent(normalizeCommunityRelayUrl(relayUrl))}';

/// The earlier key, shared by every community of the identity.
String legacyThreadFollowsKey(String pubkey) =>
    'buzz-thread-follows.v1:$pubkey';

class ThreadFollowEntry {
  final String rootId;
  final int followedAt;

  const ThreadFollowEntry({required this.rootId, required this.followedAt});

  Map<String, dynamic> toJson() => {'rootId': rootId, 'followedAt': followedAt};
}

class ThreadFollowsStorage {
  final SharedPreferences _prefs;

  ThreadFollowsStorage(this._prefs);

  List<ThreadFollowEntry> read(String pubkey, String relayUrl) {
    final key = threadFollowsKey(pubkey, relayUrl);
    // Each community starts from the shared entry, which stays for the
    // others. Its root ids are event ids, so they only ever match threads in
    // their own community, and as the oldest follows they are evicted first.
    final raw =
        _prefs.getString(key) ??
        _prefs.getString(legacyThreadFollowsKey(pubkey));
    if (raw == null || raw.isEmpty) return const [];

    try {
      final parsed = jsonDecode(raw);
      if (parsed is! List) return const [];
      return [
        for (final entry in parsed)
          if (entry is Map &&
              entry['rootId'] is String &&
              entry['followedAt'] is int)
            ThreadFollowEntry(
              rootId: entry['rootId'] as String,
              followedAt: entry['followedAt'] as int,
            ),
      ];
    } catch (_) {
      return const [];
    }
  }

  void write(String pubkey, String relayUrl, List<ThreadFollowEntry> entries) {
    _prefs.setString(
      threadFollowsKey(pubkey, relayUrl),
      jsonEncode([for (final entry in entries) entry.toJson()]),
    );
  }
}
