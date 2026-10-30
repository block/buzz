part of '../channel_messages_provider.dart';

/// System notices a window reads below its head. Past it, older pages
/// show without their notices.
const _maxNotices = 2000;

extension _ChannelNotices on ChannelMessagesNotifier {
  /// Reads the window's system notices newest first, down to its oldest
  /// loaded row. Pages leave notices out (see
  /// [EventKind.channelWindowPageKinds]), so the messages show first and the
  /// notices between them follow. Each page read calls this again to continue
  /// the descent, and a call during a read runs it once more after; a failed
  /// read leaves notices out until then.
  Future<void> _fillNotices() async {
    final initVersion = _initVersion;
    if (!_usingChannelWindow) return;
    if (_noticeFillVersion == initVersion) {
      _noticeFillAgain = true;
      return;
    }
    _noticeFillVersion = initVersion;
    _noticeFillAgain = false;
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
          throw Exception('System notices are outside their channel.');
        }
        // Pages carry their rows' reactions; notices read apart need their
        // own. Read them before the notices count as read, so a failure
        // reads the whole page again.
        final aux = await _readNoticeAux(session, events);
        if (!_isCurrentInit(initVersion)) return;
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
            throw Exception('System notices did not advance.');
          }
          _noticeCursor = ChannelPageCursor(
            createdAt: last.createdAt,
            eventId: last.id,
          );
        }
        for (final event in events) {
          _notices[event.id] = event;
        }
        for (final event in aux) {
          _noticeAux[event.id] = event;
        }
        // Add to what shows rather than replace it: the timeline can hold
        // rows the window store does not, such as live rows from before the
        // first page.
        final current =
            _historyState.value ?? _lastKnownMessages ?? const <NostrEvent>[];
        final ids = current.map((event) => event.id).toSet();
        final added = channelWindowVisibleNotices(_windowStore, [
          ...events,
          ...aux,
        ]).where((event) => ids.add(event.id)).toList();
        if (added.isEmpty) continue;
        final merged = [...current, ...added]
          ..sort(compareChannelTimelineEventsChronologically);
        _lastKnownMessages = merged;
        _historyState = AsyncData(merged);
      }
    } catch (error) {
      debugPrint(
        '[ChannelMessagesNotifier] system notices failed for $channelId: $error',
      );
    } finally {
      if (_noticeFillVersion == initVersion) {
        _noticeFillVersion = null;
        // A page that loaded during a failed read still needs its notices.
        if (_noticeFillAgain && _isCurrentInit(initVersion)) {
          unawaited(_fillNotices());
        }
      }
    }
  }

  /// The reactions to [notices] and the deletions of those reactions, in the
  /// two steps a page's `include_aux` takes. Each step reads at most
  /// [NostrFilters.channelAuxLimit] events.
  Future<List<NostrEvent>> _readNoticeAux(
    RelaySessionNotifier session,
    List<NostrEvent> notices,
  ) async {
    if (notices.isEmpty) return const [];
    final aux = await session.queryRelay([
      NostrFilters.channelAux([for (final event in notices) event.id]),
    ]);
    final reactions = [
      for (final event in aux)
        if (event.kind == EventKind.reaction) event.id,
    ];
    if (reactions.isEmpty) return aux;
    final removed = await session.queryRelay([
      NostrFilters.channelAux(
        reactions,
        kinds: const [EventKind.deletion, EventKind.nip29DeleteEvent],
      ),
    ]);
    return [...aux, ...removed];
  }
}
