import 'dart:convert';

import '../../shared/relay/relay.dart';

class ChannelPageCursor {
  final int createdAt;
  final String eventId;

  const ChannelPageCursor({required this.createdAt, required this.eventId});
}

class ChannelWindowThreadSummary {
  final int replyCount;
  final int descendantCount;
  final int? lastReplyAt;
  final List<String> participantPubkeys;

  const ChannelWindowThreadSummary({
    required this.replyCount,
    required this.descendantCount,
    required this.lastReplyAt,
    required this.participantPubkeys,
  });
}

class ChannelWindowRow {
  final NostrEvent event;
  final ChannelWindowThreadSummary? thread;

  const ChannelWindowRow({required this.event, this.thread});

  ChannelWindowRow copyWith({ChannelWindowThreadSummary? thread}) =>
      ChannelWindowRow(event: event, thread: thread ?? this.thread);
}

class ChannelWindowPage {
  final ChannelPageCursor? startCursor;
  final List<ChannelWindowRow> rows;
  final List<NostrEvent> aux;
  final ChannelPageCursor? nextCursor;
  final bool hasMore;

  const ChannelWindowPage({
    required this.startCursor,
    required this.rows,
    required this.aux,
    required this.nextCursor,
    required this.hasMore,
  });
}

class ChannelWindowStore {
  final List<ChannelWindowPage> pages;
  final List<NostrEvent> liveOverlay;
  final List<NostrEvent> liveAux;

  const ChannelWindowStore({
    required this.pages,
    required this.liveOverlay,
    required this.liveAux,
  });

  const ChannelWindowStore.empty()
    : pages = const [],
      liveOverlay = const [],
      liveAux = const [];
}

ChannelWindowPage parseChannelWindowResponse(
  List<NostrEvent> events,
  String channelId,
  ChannelPageCursor? startCursor,
) {
  var rows = [
    for (final event in events)
      if (EventKind.channelTimelineContentKinds.contains(event.kind))
        ChannelWindowRow(event: event),
  ];
  final rowIndexesById = <String, int>{
    for (var i = 0; i < rows.length; i++) rows[i].event.id: i,
  };

  for (final event in events) {
    if (event.kind != EventKind.channelThreadSummary) continue;
    final rootId = event.getTagValue('e');
    final rowIndex = rootId == null ? null : rowIndexesById[rootId];
    if (rowIndex == null) continue;
    final payload = _parseJsonMap(event, 'thread summary');
    final participants = payload['participants'];
    rows[rowIndex] = rows[rowIndex].copyWith(
      thread: ChannelWindowThreadSummary(
        replyCount: (payload['reply_count'] as num).toInt(),
        descendantCount: (payload['descendant_count'] as num).toInt(),
        lastReplyAt: (payload['last_reply_at'] as num?)?.toInt(),
        participantPubkeys: participants is List
            ? participants.whereType<String>().toList()
            : const [],
      ),
    );
  }

  final boundsEvents = events
      .where((event) => event.kind == EventKind.channelWindowBounds)
      .toList();
  if (boundsEvents.length != 1) {
    throw Exception(
      'Channel window response must contain exactly one bounds event.',
    );
  }
  final boundsEvent = boundsEvents.single;
  if (boundsEvent.getTagValue('d') !=
      _expectedBoundsKey(channelId, startCursor)) {
    throw Exception('Channel window bounds do not match the request cursor.');
  }

  final bounds = _parseJsonMap(boundsEvent, 'window bounds');
  final hasMore = bounds['has_more'] as bool;
  final nextCursor = _parseCursor(bounds['next_cursor']);
  if (hasMore != (nextCursor != null)) {
    throw Exception('Channel window bounds has_more and next_cursor disagree.');
  }

  return ChannelWindowPage(
    startCursor: startCursor,
    rows: rows,
    aux: [
      for (final event in events)
        if (EventKind.channelAuxEventKinds.contains(event.kind)) event,
    ],
    nextCursor: nextCursor,
    hasMore: hasMore,
  );
}

ChannelWindowStore replaceNewestChannelWindow(
  ChannelWindowStore current,
  ChannelWindowPage page,
) {
  if (page.startCursor != null) {
    throw Exception('Newest channel page must have a null start cursor.');
  }
  _assertValidPage(page);
  final rowIds = page.rows.map((row) => row.event.id).toSet();
  final auxIds = page.aux.map((event) => event.id).toSet();
  return ChannelWindowStore(
    pages: [page],
    liveOverlay: current.liveOverlay
        .where((event) => !rowIds.contains(event.id))
        .toList(),
    liveAux: current.liveAux
        .where((event) => !auxIds.contains(event.id))
        .toList(),
  );
}

ChannelWindowStore appendOlderChannelWindow(
  ChannelWindowStore current,
  ChannelWindowPage page,
) {
  _assertValidPage(page);
  if (current.pages.isEmpty) {
    throw Exception('Load the newest channel page first.');
  }
  final tail = current.pages.last;
  if (!tail.hasMore || tail.nextCursor == null) {
    throw Exception('The channel window is already complete.');
  }
  if (!_cursorsEqual(page.startCursor, tail.nextCursor)) {
    throw Exception(
      'Channel page does not continue the retained cursor chain.',
    );
  }
  final retainedIds = current.pages
      .expand((page) => page.rows)
      .map((row) => row.event.id)
      .toSet();
  for (final row in page.rows) {
    if (retainedIds.contains(row.event.id)) {
      throw Exception('Channel row ${row.event.id} overlaps a retained page.');
    }
  }
  final pageIds = page.rows.map((row) => row.event.id).toSet();
  return ChannelWindowStore(
    pages: [...current.pages, page],
    liveOverlay: current.liveOverlay
        .where((event) => !pageIds.contains(event.id))
        .toList(),
    liveAux: current.liveAux,
  );
}

ChannelWindowStore mergeLiveChannelWindowEvent(
  ChannelWindowStore current,
  NostrEvent event, {
  required bool isTimelineRow,
}) {
  if (!isTimelineRow) {
    final alreadyKnown =
        current.liveAux.any((candidate) => candidate.id == event.id) ||
        current.pages.any(
          (page) => page.aux.any((candidate) => candidate.id == event.id),
        );
    if (alreadyKnown) return current;
    return ChannelWindowStore(
      pages: current.pages,
      liveOverlay: current.liveOverlay,
      liveAux: [...current.liveAux, event],
    );
  }

  final inPages = current.pages.any(
    (page) => page.rows.any((row) => row.event.id == event.id),
  );
  if (inPages) return current;
  final oldestPage = current.pages.isEmpty ? null : current.pages.last;
  final oldest = oldestPage?.rows.isEmpty ?? true
      ? null
      : oldestPage!.rows.last.event;
  if (oldest != null && _compareRelayOrder(event, oldest) >= 0) return current;
  final overlay =
      current.liveOverlay
          .where((candidate) => candidate.id != event.id)
          .toList()
        ..add(event)
        ..sort(_compareRelayOrder);
  return ChannelWindowStore(
    pages: current.pages,
    liveOverlay: overlay,
    liveAux: current.liveAux,
  );
}

List<NostrEvent> flattenChannelWindowEvents(ChannelWindowStore store) {
  final byId = <String, NostrEvent>{};
  for (final page in store.pages) {
    for (final row in page.rows) {
      byId[row.event.id] = row.event;
    }
    for (final event in page.aux) {
      byId[event.id] = event;
    }
  }
  for (final event in store.liveOverlay) {
    byId[event.id] = event;
  }
  for (final event in store.liveAux) {
    byId[event.id] = event;
  }
  return byId.values.toList()
    ..sort((left, right) => _compareRelayOrder(right, left));
}

bool channelWindowHasMore(ChannelWindowStore store) =>
    store.pages.isNotEmpty && store.pages.last.hasMore;

ChannelPageCursor? channelWindowNextCursor(ChannelWindowStore store) =>
    store.pages.isEmpty ? null : store.pages.last.nextCursor;

Map<String, ChannelWindowThreadSummary> channelWindowThreadSummaries(
  ChannelWindowStore store,
) {
  return {
    for (final page in store.pages)
      for (final row in page.rows)
        if (row.thread != null) row.event.id: row.thread!,
  };
}

Map<String, dynamic> _parseJsonMap(NostrEvent event, String label) {
  try {
    final decoded = jsonDecode(event.content);
    if (decoded is Map<String, dynamic>) return decoded;
  } catch (_) {}
  throw Exception('Invalid $label event ${event.id}.');
}

ChannelPageCursor? _parseCursor(Object? value) {
  if (value == null) return null;
  if (value is! Map<String, dynamic>) {
    throw Exception('Invalid channel window cursor.');
  }
  return ChannelPageCursor(
    createdAt: (value['created_at'] as num).toInt(),
    eventId: value['id'] as String,
  );
}

String _expectedBoundsKey(String channelId, ChannelPageCursor? cursor) {
  final suffix = cursor == null
      ? 'head'
      : '${cursor.createdAt}:${cursor.eventId.toLowerCase()}';
  return '${channelId.toLowerCase()}:$suffix';
}

bool _cursorsEqual(ChannelPageCursor? left, ChannelPageCursor? right) =>
    identical(left, right) ||
    (left != null &&
        right != null &&
        left.createdAt == right.createdAt &&
        left.eventId == right.eventId);

void _assertValidPage(ChannelWindowPage page) {
  if (page.hasMore != (page.nextCursor != null)) {
    throw Exception('Channel window hasMore and nextCursor disagree.');
  }
  final seen = <String>{};
  for (var index = 0; index < page.rows.length; index++) {
    final event = page.rows[index].event;
    if (!seen.add(event.id)) {
      throw Exception('Duplicate channel row ${event.id}.');
    }
    final startCursor = page.startCursor;
    if (startCursor != null && !_isStrictlyOlder(event, startCursor)) {
      throw Exception(
        'Channel row ${event.id} is outside its cursor interval.',
      );
    }
    if (index > 0 &&
        _compareRelayOrder(page.rows[index - 1].event, event) > 0) {
      throw Exception('Channel window rows are not in relay order.');
    }
  }
}

bool _isStrictlyOlder(NostrEvent event, ChannelPageCursor cursor) =>
    event.createdAt < cursor.createdAt ||
    (event.createdAt == cursor.createdAt &&
        event.id.compareTo(cursor.eventId) > 0);

int _compareRelayOrder(NostrEvent left, NostrEvent right) {
  if (left.createdAt != right.createdAt) {
    return right.createdAt - left.createdAt;
  }
  return left.id.compareTo(right.id);
}
