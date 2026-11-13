import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../read_state/read_state_time.dart';
import '../relay/relay.dart';
import 'custom_emoji.dart';

/// Community custom-emoji palette (NIP-30, per-user kind:30030 sets unioned).
///
/// On build, fetches every member's set and collapses to one entry per
/// shortcode (see [unionCustomEmoji]). Re-fetches when the relay session
/// reconnects, asking only for sets written since shortly before the previous
/// read: the sets are a megabyte on a busy community and rarely change. The
/// palette is the single source of truth consumed by the renderer, picker,
/// autocomplete, and reaction/send tag plumbing.
class CustomEmojiPaletteNotifier extends AsyncNotifier<List<CustomEmoji>> {
  RelayClient? _client;

  /// The latest set read per author, for this [_client].
  final Map<String, NostrEvent> _sets = {};

  /// When the last complete read was sent.
  int? _readAt;

  @override
  Future<List<CustomEmoji>> build() {
    final client = ref.watch(relayClientProvider);
    ref.watch(relaySessionProvider.select((state) => state.status));
    if (!identical(client, _client)) {
      _client = client;
      _sets.clear();
      _readAt = null;
    }
    return _fetch();
  }

  Future<List<CustomEmoji>> _fetch() async {
    final sessionState = ref.read(relaySessionProvider);
    if (sessionState.status != SessionStatus.connected) {
      return _sets.isEmpty ? [] : unionCustomEmoji(_sets.values);
    }

    final client = _client;
    final previousRead = _readAt;
    final sentAt = currentUnixSeconds();
    try {
      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.fetchHistory(
        NostrFilter(
          kinds: const [kindEmojiSet],
          tags: const {
            '#d': [customEmojiSetDTag],
          },
          // Another device can store a set backdated up to this far, so a
          // repeated read reaches back past the previous one.
          since: previousRead == null
              ? null
              : previousRead - relayCrossDeviceSkewSeconds,
          // One 30030 per member; the relay keeps only the latest per
          // (pubkey, d_tag), so this bounds member count, not history.
          limit: 500,
        ),
      );
      if (identical(client, _client)) {
        for (final event in events) {
          final known = _sets[event.pubkey];
          if (known == null || event.createdAt > known.createdAt) {
            _sets[event.pubkey] = event;
          }
        }
        _readAt = sentAt;
      }
      return unionCustomEmoji(_sets.values);
    } catch (_) {
      return _sets.isEmpty ? [] : unionCustomEmoji(_sets.values);
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
