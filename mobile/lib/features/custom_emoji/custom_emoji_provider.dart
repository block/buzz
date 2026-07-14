import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import 'custom_emoji.dart';

/// Community custom-emoji palette (NIP-30, per-user kind:30030 sets unioned).
///
/// On build, fetches every member's set and collapses to one entry per
/// shortcode (see [unionCustomEmoji]). Re-fetches when the relay session
/// reconnects. The palette is the single source of truth consumed by the
/// renderer, picker, autocomplete, and reaction/send tag plumbing.
class CustomEmojiPaletteNotifier extends AsyncNotifier<List<CustomEmoji>> {
  @override
  Future<List<CustomEmoji>> build() {
    ref.watch(relayClientProvider);
    ref.watch(relaySessionProvider);
    return _fetch();
  }

  Future<List<CustomEmoji>> _fetch() async {
    final sessionState = ref.read(relaySessionProvider);
    if (sessionState.status != SessionStatus.connected) return [];

    try {
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
    } catch (_) {
      return [];
    }
  }

  /// Force a re-fetch of the community palette.
  Future<void> refresh() async {
    state = const AsyncValue.loading();
    state = await AsyncValue.guard(_fetch);
  }
}

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
