import '../../shared/relay/relay.dart';
import '../channels/channel_window.dart';
import 'forum_auxiliary.dart';

final _editEventId = RegExp(r'^[0-9a-fA-F]{64}$');

/// Maximum retained history, including rows merged by periodic head refreshes.
const forumPageLimit = 200;
const _maxRows = forumPageLimit * 50;
const _maxAux = forumPageLimit * 2000;

/// Forum-only completeness and effect scopes for a fetched page.
class ForumWindowPage extends ChannelWindowPage {
  /// Rows whose page had unconfirmed history or auxiliary authority/closure.
  final Set<String> incompletePostIds;

  /// Deletion effects checked against these targets without relay authority.
  /// Missing entries represent relay-authorized served-window events.
  final Map<String, Set<String>> auxiliaryTargets;

  /// Unconfirmed root/page effects without a retained row to mark.
  final bool hasUnconfirmedRootEffects;

  const ForumWindowPage({
    required super.startCursor,
    required super.rows,
    required super.aux,
    required super.nextCursor,
    required super.hasMore,
    this.incompletePostIds = const {},
    this.auxiliaryTargets = const {},
    this.hasUnconfirmedRootEffects = false,
  });
}

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
  final rowIds = rows.map((row) => row.event.id).toSet();
  final aux = {
    for (final p in current.pages)
      for (final event in p.aux) event.id: event,
    for (final event in page.aux) event.id: event,
  }.values.toList();
  final incompletePostIds = {
    for (final previous in current.pages)
      if (previous is ForumWindowPage) ...previous.incompletePostIds,
  }..removeAll(incoming.keys);
  incompletePostIds.retainAll(rowIds);
  if (page is ForumWindowPage) {
    incompletePostIds.addAll(page.incompletePostIds);
  }
  var hasUnconfirmedRootEffects =
      page is ForumWindowPage && page.hasUnconfirmedRootEffects;
  for (final event in page.aux) {
    if (forumEditTargetsAgree(event)) continue;
    final last = event.tags
        .where((tag) => tag.length >= 2 && tag[0] == 'e')
        .lastOrNull;
    // Both the relay-authorized and formatter-applied targets may be stale.
    // Capture their uncertainty on receipt, so retained raw aux cannot revive
    // warnings after each affected row has been reread or removed.
    for (final target in [_firstEditTarget(event), last?[1].toLowerCase()]) {
      if (target != null && rowIds.contains(target)) {
        incompletePostIds.add(target);
      } else {
        hasUnconfirmedRootEffects = true;
      }
    }
  }
  final auxiliaryTargets = <String, Set<String>>{};
  for (final source in [...current.pages, page]) {
    if (source is! ForumWindowPage) continue;
    for (final entry in source.auxiliaryTargets.entries) {
      // One deletion may have targets authorized on different loaded pages.
      auxiliaryTargets.putIfAbsent(entry.key, () => {}).addAll(entry.value);
    }
  }
  if (rows.length > _maxRows || aux.length > _maxAux) {
    throw StateError('Forum history limit reached; more history remains');
  }
  final cursor = refresh && connected && current.pages.isNotEmpty
      ? channelWindowNextCursor(current)
      : next;
  return ChannelWindowStore(
    pages: [
      ForumWindowPage(
        startCursor: null,
        rows: rows,
        aux: aux,
        nextCursor: next == null && refresh ? null : cursor,
        hasMore: (next == null && refresh ? null : cursor) != null,
        incompletePostIds: incompletePostIds,
        auxiliaryTargets: auxiliaryTargets,
        hasUnconfirmedRootEffects:
            hasUnconfirmedRootEffects ||
            (!refresh &&
                current.pages.whereType<ForumWindowPage>().any(
                  (p) => p.hasUnconfirmedRootEffects,
                )),
      ),
    ],
    liveOverlay: const [],
    liveAux: const [],
    liveThreadSummaries: const {},
  );
}

String? _firstEditTarget(NostrEvent event) {
  for (final tag in event.tags) {
    if (tag.length >= 2 &&
        tag[0] == 'e' &&
        tag[1].length == 64 &&
        _editEventId.hasMatch(tag[1])) {
      return tag[1].toLowerCase();
    }
  }
  return null;
}

int _order(NostrEvent a, NostrEvent b) {
  final time = b.createdAt.compareTo(a.createdAt);
  return time == 0 ? a.id.compareTo(b.id) : time;
}
