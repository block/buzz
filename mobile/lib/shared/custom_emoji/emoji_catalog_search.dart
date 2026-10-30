import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/relay.dart';
import 'custom_emoji.dart';
import 'emoji_entry.dart';

/// Where a catalog search stands. [results] belong to [query] and stay while a
/// newer search runs or fails, so the grid does not blank.
@immutable
class EmojiCatalogSearchState {
  final String? query;
  final bool loading;
  final bool failed;
  final List<CustomEmoji> results;

  const EmojiCatalogSearchState({
    this.query,
    this.loading = false,
    this.failed = false,
    this.results = const [],
  });
}

/// Prefix search over the relay's kind:30180 catalog.
///
/// Each call gets a request number. A reply that is no longer the newest
/// request changes nothing, so a slow answer for an old query cannot replace
/// the answer for the current one.
class EmojiCatalogSearchNotifier extends Notifier<EmojiCatalogSearchState> {
  int _request = 0;

  @override
  EmojiCatalogSearchState build() {
    ref.watch(relayConfigProvider);
    ref.onDispose(() => _request++);
    return const EmojiCatalogSearchState();
  }

  /// Search for [text], which [emojiCatalogSearchText] already normalized.
  Future<void> search(String text) async {
    final request = ++_request;
    state = EmojiCatalogSearchState(
      query: text,
      loading: true,
      results: state.results,
    );
    try {
      final events = await ref.read(relaySessionProvider.notifier).queryRelay([
        NostrFilters.emojiEntrySearch(text, limit: maxEmojiCatalogResults),
      ]);
      if (request != _request) return;
      final resolution = resolveEmojiEntries(events);
      if (resolution.conflicts.isNotEmpty) {
        debugPrint(
          'emoji catalog: several authors published '
          '${resolution.conflicts.join(', ')}; using the newest',
        );
      }
      state = EmojiCatalogSearchState(query: text, results: resolution.emoji);
    } catch (error) {
      if (request != _request) return;
      debugPrint('emoji catalog search failed: $error');
      state = EmojiCatalogSearchState(
        query: text,
        failed: true,
        results: state.results,
      );
    }
  }

  /// Run the last query again.
  Future<void> retry() {
    final query = state.query;
    return query == null ? Future.value() : search(query);
  }

  /// Drop the results and ignore any search still in flight.
  void clear() {
    _request++;
    state = const EmojiCatalogSearchState();
  }
}

final emojiCatalogSearchProvider =
    NotifierProvider.autoDispose<
      EmojiCatalogSearchNotifier,
      EmojiCatalogSearchState
    >(EmojiCatalogSearchNotifier.new);

/// Catalog emoji the user picked in this session, newest first.
///
/// A message or reaction carries its emoji URL from the palette, so a picked
/// catalog emoji joins [customEmojiListProvider] through this list. The most
/// recent pick for a shortcode wins over the legacy palette.
class PickedCatalogEmojiNotifier extends Notifier<List<CustomEmoji>> {
  @override
  List<CustomEmoji> build() {
    ref.watch(relayConfigProvider);
    return const [];
  }

  void add(CustomEmoji emoji) {
    state = [
      emoji,
      for (final other in state)
        if (other.shortcode != emoji.shortcode) other,
    ].take(maxPickedCatalogEmoji).toList();
  }
}

/// Picks kept per session; older ones fall out.
const int maxPickedCatalogEmoji = 200;

final pickedCatalogEmojiProvider =
    NotifierProvider<PickedCatalogEmojiNotifier, List<CustomEmoji>>(
      PickedCatalogEmojiNotifier.new,
    );
