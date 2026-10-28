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

    // Retained notification targets are isolated rows, not a history page.
    final oldest = _legacyHistoryCursor;
    if (oldest == null) return false;
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
    final preceding =
        older
            .where(
              (event) =>
                  compareChannelTimelineEventsChronologically(event, oldest) <
                  0,
            )
            .toList()
          ..sort(compareChannelTimelineEventsChronologically);
    if (preceding.isEmpty) return false;
    _legacyHistoryCursor = preceding.first;
    final currentIds = _historyState.value?.map((e) => e.id).toSet() ?? {};
    final deduped = preceding.where((e) => !currentIds.contains(e.id)).toList();
    // Advance even when a page consists entirely of previously retained rows.
    _historyState = _historyState.whenData((events) {
      final merged = [...deduped, ...events];
      merged.sort(compareChannelTimelineEventsChronologically);
      _lastKnownMessages = merged;
      return merged;
    });
    return true;
  }
}
