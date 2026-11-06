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
