import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

const inboxNotificationSeenStorageKey = 'buzz-home-feed-seen.v1';
const inboxNotificationSeenMaxItems = 500;

String inboxNotificationSeenPrefsKey(String pubkey) {
  return '$inboxNotificationSeenStorageKey:${pubkey.toLowerCase()}';
}

List<String> readStoredInboxNotificationSeenIds(
  SharedPreferences prefs,
  String pubkey,
) {
  final normalized = pubkey.trim().toLowerCase();
  if (normalized.isEmpty) return const [];

  final raw = prefs.getString(inboxNotificationSeenPrefsKey(normalized));
  if (raw == null || raw.isEmpty) return const [];

  try {
    final decoded = jsonDecode(raw);
    if (decoded is! List) return const [];
    return [
      for (final value in decoded)
        if (value is String) value,
    ].take(inboxNotificationSeenMaxItems).toList();
  } catch (_) {
    return const [];
  }
}

Future<void> writeStoredInboxNotificationSeenIds(
  SharedPreferences prefs,
  String pubkey,
  Iterable<String> ids,
) async {
  final normalized = pubkey.trim().toLowerCase();
  if (normalized.isEmpty) return;

  final capped = ids.toList();
  if (capped.length > inboxNotificationSeenMaxItems) {
    capped.removeRange(0, capped.length - inboxNotificationSeenMaxItems);
  }
  await prefs.setString(
    inboxNotificationSeenPrefsKey(normalized),
    jsonEncode(capped),
  );
}

Set<String> capSeenInboxNotificationIds(Set<String> ids) {
  if (ids.length <= inboxNotificationSeenMaxItems) return ids;
  final list = ids.toList();
  return list.sublist(list.length - inboxNotificationSeenMaxItems).toSet();
}
