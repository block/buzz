import 'dart:convert';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../theme/theme_provider.dart';

// v1 allowed unbounded rasters and v2 allowed arbitrary SVG resources.
// Rebuild this derived cache through the validated artwork pipeline.
const _storageKey = 'buzz.community-icons.v3';
const _maximumEntries = 32;
const _maximumStoredCharacters = 3 * 1024 * 1024;

/// Last downloaded public icon artwork, keyed by the relay information URL.
/// It intentionally survives active-community changes and app restarts.
final communityIconCacheProvider =
    NotifierProvider<CommunityIconCache, Map<String, String>>(
      CommunityIconCache.new,
    );

/// A bounded, best-effort local cache; it never stores community credentials.
class CommunityIconCache extends Notifier<Map<String, String>> {
  SharedPreferences? _prefs;
  Future<void> _writes = Future.value();

  @override
  Map<String, String> build() {
    try {
      _prefs = ref.read(savedPrefsProvider);
      final raw = _prefs?.getString(_storageKey);
      if (raw == null || raw.length > _maximumStoredCharacters) return {};
      final decoded = jsonDecode(raw);
      if (decoded is! Map<String, dynamic>) return {};
      return _bounded({
        for (final entry in decoded.entries)
          if (entry.value is String &&
              (entry.value as String).startsWith('data:image/'))
            entry.key: entry.value as String,
      });
    } catch (_) {
      // Artwork caching is optional when preferences are unavailable/corrupt.
      return {};
    }
  }

  /// Stores complete artwork, or clears an explicitly removed relay icon.
  Future<void> remember(String relay, String? artwork) {
    if (state[relay] == artwork) return _writes;
    final next = {...state}..remove(relay);
    if (artwork != null) next[relay] = artwork;
    state = _bounded(next);
    final encoded = jsonEncode(state);
    _writes = _writes.then((_) async {
      try {
        await _prefs?.setString(_storageKey, encoded);
      } catch (_) {
        // Retain the in-memory copy even if persistence is unavailable.
      }
    });
    return _writes;
  }

  Map<String, String> _bounded(Map<String, String> entries) {
    var size = jsonEncode(entries).length;
    while (entries.length > _maximumEntries ||
        size > _maximumStoredCharacters) {
      entries.remove(entries.keys.first);
      size = jsonEncode(entries).length;
    }
    return Map.unmodifiable(entries);
  }
}
