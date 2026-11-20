import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../channels/channel_window.dart';
import 'forum_models.dart';
import 'forum_window.dart';

/// A scope-captured reader and its forum-specific presentation.
abstract interface class ForumSource<T extends ForumReadResponse> {
  /// Reads a head or continuation page for this captured scope.
  Future<ChannelWindowPage> fetch(ChannelPageCursor? cursor);

  /// Applies auxiliary events and exposes completeness and retry state.
  T render(
    ChannelWindowStore store, {
    bool loadingMore = false,
    Object? loadMoreError,
    Object? refreshError,
  });
}

/// Shared bounded pagination, head-only refresh, and generation fencing.
abstract class ForumReader<T extends ForumReadResponse>
    extends AsyncNotifier<T> {
  ChannelWindowStore _store = const ChannelWindowStore.empty();
  ForumSource<T>? _source;
  int _generation = 0;
  int _pages = 0;
  bool _busy = false;

  /// Captures the active scope; only thread sources validate a root here.
  /// [checkActive] must fence requests and asynchronous verification results.
  Future<ForumSource<T>> open(void Function() checkActive);

  @override
  Future<T> build() async {
    final generation = ++_generation;
    ref.onDispose(() => _generation++);
    _busy = true;
    void checkActive() {
      if (!ref.mounted || generation != _generation) {
        throw StateError('Forum read superseded');
      }
    }

    final source = await open(checkActive);
    if (!ref.mounted || generation != _generation) {
      throw StateError('Forum read superseded');
    }
    final page = await source.fetch(null);
    if (!ref.mounted || generation != _generation) {
      throw StateError('Forum read superseded');
    }
    final store = mergeForumWindow(const ChannelWindowStore.empty(), page);
    final response = source.render(store);
    _source = source;
    _store = store;
    _pages = 1;
    _busy = false;
    return response;
  }

  /// Refreshes one newest page, retaining older rows and the continuation.
  Future<void> refresh() async {
    if (_busy && !state.hasError) return;
    if (_source == null || state.hasError) {
      ref.invalidateSelf();
      return;
    }
    final generation = _generation;
    final source = _source!;
    final loadMoreError = state.value?.loadMoreError;
    _busy = true;
    try {
      final page = await source.fetch(null);
      if (!ref.mounted || generation != _generation) return;
      final store = mergeForumWindow(_store, page, refresh: true);
      final next = channelWindowNextCursor(store);
      // A head gap can change the cursor without settling a failed older read.
      final response = source.render(
        store,
        loadMoreError: next == null ? null : loadMoreError,
      );
      _store = store;
      state = AsyncData(response);
    } catch (error, stack) {
      if (!ref.mounted || generation != _generation) return;
      if (error is ForumPostDeleted) {
        state = AsyncError(error, stack);
      } else {
        state = AsyncData(
          source.render(
            _store,
            loadMoreError: loadMoreError,
            refreshError: error,
          ),
        );
      }
    } finally {
      if (generation == _generation) _busy = false;
    }
  }

  /// Continues from the last validated cursor. Failures keep it retryable.
  Future<void> loadMore() async {
    final response = state.value;
    if (_busy || response == null || !response.hasMore) return;
    final source = _source!;
    final generation = _generation;
    _busy = true;
    final refreshError = response.refreshError;
    state = AsyncData(
      source.render(_store, loadingMore: true, refreshError: refreshError),
    );
    try {
      if (_pages >= forumPageLimit) {
        throw StateError('Forum page limit reached; history is incomplete');
      }
      final page = await source.fetch(channelWindowNextCursor(_store));
      if (!ref.mounted || generation != _generation) return;
      final store = mergeForumWindow(_store, page);
      final next = source.render(store, refreshError: refreshError);
      _store = store;
      _pages++;
      state = AsyncData(next);
    } catch (error, stack) {
      if (!ref.mounted || generation != _generation) return;
      if (error is ForumPostDeleted) {
        state = AsyncError(error, stack);
      } else {
        state = AsyncData(
          source.render(
            _store,
            loadMoreError: error,
            refreshError: refreshError,
          ),
        );
      }
    } finally {
      if (generation == _generation) _busy = false;
    }
  }
}

/// Server-declared deadlines pause automatic polling even with retained rows.
bool forumPollingPaused(AsyncValue<ForumReadResponse> state) =>
    isSettledRelayDeadline(state) ||
    (state.value?.refreshError != null &&
        isRelayDeadlineError(state.value!.refreshError!));
