import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/relay.dart';
import 'custom_emoji.dart';

/// Community custom-emoji palette (NIP-30, per-user kind:30030 sets unioned).
///
/// On build, fetches every member's set and collapses to one entry per
/// shortcode (see [unionCustomEmoji]). Re-fetches when the relay session
/// reconnects. The palette is the single source of truth consumed by the
/// renderer, picker, autocomplete, and reaction/send tag plumbing.
class CustomEmojiPaletteNotifier extends AsyncNotifier<List<CustomEmoji>> {
  List<CustomEmoji>? _lastGood;
  String? _lastGoodRelayUrl;
  int _generation = 0;

  @override
  Future<List<CustomEmoji>> build() async {
    ref.watch(relayClientProvider);
    ref.watch(relaySessionProvider);
    final relayUrl = ref.watch(relayConfigProvider.select((c) => c.baseUrl));
    if (relayUrl != _lastGoodRelayUrl) {
      final switched = _lastGoodRelayUrl != null;
      _lastGood = null;
      _lastGoodRelayUrl = relayUrl;
      if (switched) {
        state = const AsyncData([]);
        Future.microtask(() => _setRefreshFailed(false));
      }
    }
    final generation = ++_generation;
    if (ref.read(relaySessionProvider).status != SessionStatus.connected) {
      return _lastGood ?? const [];
    }
    try {
      final palette = await _query();
      if (generation != _generation) return _lastGood ?? palette;
      _lastGood = palette;
      _setRefreshFailed(false);
      return palette;
    } catch (_) {
      final lastGood = _lastGood;
      if (lastGood == null) rethrow;
      if (generation == _generation) _setRefreshFailed(true);
      return lastGood;
    }
  }

  Future<List<CustomEmoji>> _query() async {
    final session = ref.read(relaySessionProvider.notifier);
    final events = await session.fetchHistory(
      const NostrFilter(
        kinds: [kindEmojiSet],
        tags: {
          '#d': [customEmojiSetDTag],
        },
        // One 30030 per member; the relay keeps only the latest per
        // (pubkey, d_tag), so this bounds member count, not history.
        limit: 500,
      ),
    );
    return unionCustomEmoji(events);
  }

  void _setRefreshFailed(bool failed) =>
      ref.read(customEmojiRefreshFailedProvider.notifier).set(failed);

  /// Re-fetch the community palette. The current palette stays visible while
  /// the fetch runs, and a failed fetch keeps the last good palette and raises
  /// [customEmojiRefreshFailedProvider].
  Future<void> refresh() async {
    final generation = ++_generation;
    try {
      final palette = await _query();
      if (generation != _generation) return;
      _lastGood = palette;
      _setRefreshFailed(false);
      state = AsyncData(palette);
    } catch (error, stackTrace) {
      if (generation != _generation) return;
      if (_lastGood == null) {
        state = AsyncError(error, stackTrace);
      } else {
        _setRefreshFailed(true);
      }
    }
  }
}

/// Whether the latest palette fetch failed while an older palette is still
/// being shown.
class CustomEmojiRefreshFailedNotifier extends Notifier<bool> {
  @override
  bool build() => false;

  void set(bool failed) => state = failed;
}

final customEmojiRefreshFailedProvider =
    NotifierProvider<CustomEmojiRefreshFailedNotifier, bool>(
      CustomEmojiRefreshFailedNotifier.new,
    );

final customEmojiPaletteProvider =
    AsyncNotifierProvider<CustomEmojiPaletteNotifier, List<CustomEmoji>>(
      CustomEmojiPaletteNotifier.new,
    );

/// Synchronous read of the current palette (empty while loading/error).
/// Convenience for widgets that only need the resolved list, not the async
/// state — renderer, autocomplete, reaction resolution.
final customEmojiListProvider = Provider<List<CustomEmoji>>((ref) {
  return ref.watch(customEmojiPaletteProvider).value ?? const [];
});
