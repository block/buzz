import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

/// Persistence for followed thread root IDs.
///
/// Mirrors the desktop store (`useThreadFollows.ts`): per-pubkey entries of
/// `{rootId, followedAt}`, capped at the oldest-out limit. Desktop keeps this
/// client-local (localStorage), so mobile does the same with
/// SharedPreferences — following on one device intentionally does not sync.
const threadFollowsMaxEntries = 500;

String threadFollowsKey(String pubkey) => 'buzz-thread-follows.v1:$pubkey';

class ThreadFollowEntry {
  final String rootId;
  final int followedAt;

  const ThreadFollowEntry({required this.rootId, required this.followedAt});

  Map<String, dynamic> toJson() => {'rootId': rootId, 'followedAt': followedAt};
}

class ThreadFollowsStorage {
  final SharedPreferences _prefs;

  ThreadFollowsStorage(this._prefs);

  List<ThreadFollowEntry> read(String pubkey) {
    final raw = _prefs.getString(threadFollowsKey(pubkey));
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

  void write(String pubkey, List<ThreadFollowEntry> entries) {
    _prefs.setString(
      threadFollowsKey(pubkey),
      jsonEncode([for (final entry in entries) entry.toJson()]),
    );
  }
}

/// Cap entries at [threadFollowsMaxEntries], evicting the oldest follows
/// first — same policy as desktop's `capEntries`.
List<ThreadFollowEntry> capThreadFollowEntries(
  List<ThreadFollowEntry> entries,
) {
  if (entries.length <= threadFollowsMaxEntries) return entries;
  final sorted = [...entries]
    ..sort((a, b) => a.followedAt.compareTo(b.followedAt));
  return sorted.sublist(sorted.length - threadFollowsMaxEntries);
}
