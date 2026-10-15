part of '../channel_messages_provider.dart';

extension _ChannelHistoryPagination on ChannelMessagesNotifier {
  Future<bool> _fetchOlder() async {
    if (_reachedOldest || _initInFlight) return false;

    final session = _summarySession;
    if (_usingChannelWindow) {
      final cursor = channelWindowNextCursor(_windowStore);
      if (cursor == null) {
        _reachedOldest = true;
        return false;
      }
      try {
        final pageVersion = ++_threadQuerySerial;
        final page = await _fetchWindowPage(session, cursor);
        _windowStore = appendOlderChannelWindow(_windowStore, page);
        _reconcilePageSummaries(page, pageVersion);
        _reachedOldest = !channelWindowHasMore(_windowStore);
        final flattened = _withDeepLinkEvents(
          flattenChannelWindowEvents(_windowStore),
        );
        _lastKnownMessages = flattened;
        _reconcileRetainedSummaryPayloads(page.rows.map((row) => row.event));
        _historyState = AsyncData(flattened);
        return page.rows.isNotEmpty || page.aux.isNotEmpty;
      } catch (error) {
        debugPrint(
          '[ChannelMessagesNotifier] failed to fetch older channel window page for $channelId: $error',
        );
        return false;
      }
    }

    final currentEvents = _historyState.value;
    if (currentEvents == null || currentEvents.isEmpty) return false;
    final oldest = currentEvents.first;
    final older = await session.fetchHistory(
      NostrFilters.messages(
        channelId,
        limit: 100,
        until: oldest.createdAt,
        beforeId: oldest.id,
      ),
    );
    if (older.isEmpty) {
      _reachedOldest = true;
      return false;
    }
    final currentIds = _historyState.value?.map((e) => e.id).toSet() ?? {};
    final deduped = older.where((e) => !currentIds.contains(e.id)).toList();
    if (deduped.isEmpty) {
      // A repeated page is not proof that history is exhausted.
      return false;
    }
    _historyState = _historyState.whenData((events) {
      final merged = [...deduped, ...events];
      merged.sort(compareChannelTimelineEventsChronologically);
      _lastKnownMessages = merged;
      return merged;
    });
    return true;
  }
}
