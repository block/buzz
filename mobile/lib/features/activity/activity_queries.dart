import '../../shared/relay/relay.dart';

/// Complete aggregate participation evidence before marking interests cached.
/// A full page is not proof that every target lacks older participation.
Future<List<NostrEvent>> queryActivityEvidence(
  Future<List<NostrEvent>> Function(List<NostrFilter>) query,
  List<NostrFilter> filters,
) async {
  final result = <NostrEvent>[];
  for (final filter in filters) {
    NostrEvent? cursor;
    for (var count = 0; ; count++) {
      if (count == 20) {
        throw StateError('Activity participation exceeds its read budget.');
      }
      final page = await query([
        NostrFilter(
          kinds: filter.kinds,
          authors: filter.authors,
          tags: filter.tags,
          limit: filter.limit,
          until: cursor?.createdAt,
          extensions: {if (cursor != null) 'before_id': cursor.id},
        ),
      ]);
      page.sort((a, b) {
        final time = b.createdAt.compareTo(a.createdAt);
        return time != 0 ? time : a.id.compareTo(b.id);
      });
      if (cursor != null &&
          page.any(
            (e) =>
                e.createdAt > cursor!.createdAt ||
                (e.createdAt == cursor.createdAt &&
                    e.id.compareTo(cursor.id) <= 0),
          )) {
        throw StateError('Activity participation did not advance.');
      }
      result.addAll(page);
      if (result.length > 4096) {
        throw StateError('Activity participation exceeds its evidence budget.');
      }
      if (page.length < filter.limit) break;
      cursor = page.last;
    }
  }
  return result;
}

/// Fill bounded per-conversation evidence without querying each channel.
/// Drop saturated channels from subsequent aggregate pages so a busy channel
/// cannot consume the read budget before quieter conversations are reached.
Future<List<NostrEvent>> completeActivityColdWindow(
  Future<List<NostrEvent>> Function(List<NostrFilter>) query,
  NostrFilter filter,
  List<NostrEvent> firstPage,
) async {
  final remaining = filter.tags['#h']!.toSet();
  // Each full page either retires a channel or adds 500 events toward
  // the at-most 99 unsaturated events per channel. Bound both kinds of progress.
  final maxPages =
      remaining.length + (remaining.length * 99 / filter.limit).ceil() + 1;
  final counts = <String, int>{};
  final result = <NostrEvent>[];
  var page = {for (final event in firstPage) event.id: event}.values.toList()
    ..sort((a, b) {
      final time = b.createdAt.compareTo(a.createdAt);
      return time != 0 ? time : a.id.compareTo(b.id);
    });
  page = page.take(filter.limit).toList();
  for (var reads = 0; ; reads++) {
    page.sort((a, b) {
      final time = b.createdAt.compareTo(a.createdAt);
      return time != 0 ? time : a.id.compareTo(b.id);
    });
    for (final event in page) {
      final channel = event.channelId;
      if (!remaining.contains(channel)) continue;
      if (reads > 0) result.add(event);
      counts[channel!] = (counts[channel] ?? 0) + 1;
      if (counts[channel]! >= 100) remaining.remove(channel);
    }
    if (page.length < filter.limit || remaining.isEmpty) return result;
    if (reads >= maxPages) {
      throw StateError('Activity cold history exceeds its read budget.');
    }
    final cursor = page.last;
    page = await query([
      NostrFilter(
        kinds: filter.kinds,
        tags: {'#h': remaining.toList()},
        until: cursor.createdAt,
        extensions: {'before_id': cursor.id},
        limit: filter.limit,
      ),
    ]);
    if (page.any(
      (e) =>
          e.createdAt > cursor.createdAt ||
          (e.createdAt == cursor.createdAt && e.id.compareTo(cursor.id) <= 0),
    )) {
      throw StateError('Activity cold history did not advance.');
    }
  }
}
