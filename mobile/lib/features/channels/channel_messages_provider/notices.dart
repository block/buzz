part of '../channel_messages_provider.dart';

/// Membership notices a window reads below its head. Past it, older pages
/// show without their notices.
const _maxNotices = 2000;

extension _ChannelNotices on ChannelMessagesNotifier {
  /// Reads the window's membership notices newest first, down to its oldest
  /// loaded row. Pages leave notices out (see
  /// [EventKind.channelWindowPageKinds]), so the messages show first and the
  /// notices between them follow. Each page read calls this again to continue
  /// the descent; a failed read leaves notices out until then.
  Future<void> _fillNotices() async {
    final initVersion = _initVersion;
    if (!_usingChannelWindow || _noticeFillVersion == initVersion) return;
    _noticeFillVersion = initVersion;
    final session = _summarySession;
    try {
      while (_isCurrentInit(initVersion) &&
          !_noticesReachedStart &&
          _notices.length < _maxNotices) {
        final cursor = _noticeCursor;
        final floor = channelWindowFloor(_windowStore);
        // Notices below the oldest loaded row wait for the next page.
        if (cursor != null &&
            floor != null &&
            !isChannelCursorOlder(floor, cursor)) {
          break;
        }
        final events = await session.queryRelay([
          NostrFilters.channelNotices(
            channelId,
            until: cursor?.createdAt,
            beforeId: cursor?.eventId,
          ),
        ]);
        if (!_isCurrentInit(initVersion)) return;
        if (events.any(
          (event) =>
              event.kind != EventKind.systemMessage ||
              event.channelId != channelId,
        )) {
          throw Exception('Membership notices are outside their channel.');
        }
        if (events.length < NostrFilters.channelNoticePageSize) {
          _noticesReachedStart = true;
        } else {
          final last = events.reduce(
            (a, b) =>
                a.createdAt < b.createdAt ||
                    (a.createdAt == b.createdAt && a.id.compareTo(b.id) > 0)
                ? a
                : b,
          );
          if (cursor != null && !isOlderThanChannelCursor(last, cursor)) {
            throw Exception('Membership notices did not advance.');
          }
          _noticeCursor = ChannelPageCursor(
            createdAt: last.createdAt,
            eventId: last.id,
          );
        }
        for (final event in events) {
          _notices[event.id] = event;
        }
        // Add to what shows rather than replace it: the timeline can hold
        // rows the window store does not, such as live rows from before the
        // first page.
        final current =
            _historyState.value ?? _lastKnownMessages ?? const <NostrEvent>[];
        final ids = current.map((event) => event.id).toSet();
        final added = channelWindowVisibleNotices(
          _windowStore,
          events,
        ).where((event) => ids.add(event.id)).toList();
        if (added.isEmpty) continue;
        final merged = [...current, ...added]
          ..sort(compareChannelTimelineEventsChronologically);
        _lastKnownMessages = merged;
        _historyState = AsyncData(merged);
      }
    } catch (error) {
      debugPrint(
        '[ChannelMessagesNotifier] membership notices failed for $channelId: $error',
      );
    } finally {
      if (_noticeFillVersion == initVersion) _noticeFillVersion = null;
    }
  }
}
