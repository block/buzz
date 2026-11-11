import '../../shared/relay/relay.dart';
import '../channels/channel_window.dart';

/// Maximum retained history, including rows merged by periodic head refreshes.
const forumPageLimit = 200;
const _maxRows = forumPageLimit * 50;
const _maxAux = forumPageLimit * 2000;

/// Merges a head refresh or continuation by event ID without refetching history.
///
/// A head refresh removes absent rows only inside the range it actually read.
/// If a burst opens a gap above the retained history, its cursor exposes that
/// gap for manual continuation while the older loaded rows remain visible.
ChannelWindowStore mergeForumWindow(
  ChannelWindowStore current,
  ChannelWindowPage page, {
  bool refresh = false,
}) {
  // Validate ordering and cursor intervals with the shared channel machinery.
  if (page.startCursor == null) {
    replaceNewestChannelWindow(const ChannelWindowStore.empty(), page);
  } else {
    appendOlderChannelWindow(
      ChannelWindowStore(
        pages: [
          ChannelWindowPage(
            startCursor: null,
            rows: const [],
            aux: const [],
            nextCursor: page.startCursor,
            hasMore: true,
          ),
        ],
        liveOverlay: const [],
        liveAux: const [],
        liveThreadSummaries: const {},
      ),
      page,
    );
  }
  if (page.nextCursor case final next?) {
    if (page.startCursor case final start?) {
      if (!isChannelCursorOlder(next, start)) {
        throw const FormatException('Forum window cursor did not advance');
      }
    }
  }

  final previous = {
    for (final p in current.pages)
      for (final row in p.rows) row.event.id: row,
  };
  final incoming = {for (final row in page.rows) row.event.id: row};
  final oldNewest = previous.values.isEmpty
      ? null
      : (previous.values.toList()..sort((a, b) => _order(a.event, b.event)))
            .first
            .event;
  final next = page.nextCursor;
  final connected =
      next == null ||
      (oldNewest != null && !isOlderThanChannelCursor(oldNewest, next));
  if (refresh) {
    previous.removeWhere(
      (id, row) =>
          !incoming.containsKey(id) &&
          (next == null || !isOlderThanChannelCursor(row.event, next)),
    );
  }
  final rows = {...previous, ...incoming}.values.toList()
    ..sort((a, b) => _order(a.event, b.event));
  final aux = {
    for (final p in current.pages)
      for (final event in p.aux) event.id: event,
    for (final event in page.aux) event.id: event,
  }.values.toList();
  if (rows.length > _maxRows || aux.length > _maxAux) {
    throw StateError('Forum history limit reached; more history remains');
  }
  final cursor = refresh && connected && current.pages.isNotEmpty
      ? channelWindowNextCursor(current)
      : next;
  return ChannelWindowStore(
    pages: [
      ChannelWindowPage(
        startCursor: null,
        rows: rows,
        aux: aux,
        nextCursor: next == null && refresh ? null : cursor,
        hasMore: (next == null && refresh ? null : cursor) != null,
      ),
    ],
    liveOverlay: const [],
    liveAux: const [],
    liveThreadSummaries: const {},
  );
}

int _order(NostrEvent a, NostrEvent b) {
  final time = b.createdAt.compareTo(a.createdAt);
  return time == 0 ? a.id.compareTo(b.id) : time;
}
