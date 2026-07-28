import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme_provider.dart';

const _recentSearchesPrefsKey = 'recent_searches_v1';
const _maxRecentSearches = 6;

/// Device-local history of explicitly submitted searches, newest first.
///
/// Queries are scoped by community and account so searches from one identity
/// cannot appear after switching to another.
class RecentSearchesNotifier extends Notifier<List<String>> {
  late String _prefsKey;

  @override
  List<String> build() {
    final config = ref.watch(relayConfigProvider);
    final pubkey = ref.watch(myPubkeyProvider) ?? 'anon';
    _prefsKey = '$_recentSearchesPrefsKey:${config.baseUrl}:$pubkey';

    final stored =
        ref.read(savedPrefsProvider).getStringList(_prefsKey) ?? const [];
    return List.unmodifiable(
      stored
          .map((query) => query.trim())
          .where((query) => query.isNotEmpty)
          .take(_maxRecentSearches),
    );
  }

  /// Records [query] as the most recent search after trimming whitespace.
  ///
  /// Empty queries are ignored. Existing matches are deduplicated
  /// case-insensitively, the newest spelling is retained, the history is capped
  /// at six entries, and the result is persisted for the current community and
  /// account.
  void record(String query) {
    final trimmed = query.trim();
    if (trimmed.isEmpty) return;

    final normalized = trimmed.toLowerCase();
    final next = [
      trimmed,
      ...state.where((item) => item.toLowerCase() != normalized),
    ].take(_maxRecentSearches).toList(growable: false);
    _persist(next);
  }

  /// Clears the current community and account's history and persists it empty.
  void clear() => _persist(const []);

  void _persist(List<String> searches) {
    state = List.unmodifiable(searches);
    ref.read(savedPrefsProvider).setStringList(_prefsKey, searches);
  }
}

/// Provides device-local recent searches scoped to the active community and
/// account.
final recentSearchesProvider =
    NotifierProvider<RecentSearchesNotifier, List<String>>(
      RecentSearchesNotifier.new,
    );
