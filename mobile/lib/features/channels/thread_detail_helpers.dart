part of 'thread_detail_page.dart';

const _threadTailScrollTolerance = 0.5;

// Keep the direct-position correction finite in case the viewport cannot
// expose its tail (for example, continuously changing media dimensions).
const _latestTailCorrectionLimit = 8;

Widget _trackActiveThreadScrollPosition(
  Widget child,
  ObjectRef<ScrollPosition?> activePosition,
) => Builder(
  builder: (context) {
    activePosition.value = Scrollable.of(context).position;
    return child;
  },
);

/// Keeps a tail-following thread at its end while content there grows.
///
/// A sent reply's entrance transition grows its row over several frames, and
/// media or typing rows can grow too. The one-shot tail correction can finish
/// before that growth does, leaving the new content under the composer. Every
/// growth reports new scroll metrics, so move a position that was at its old
/// end to the new one while the reader follows the tail.
void _followThreadTailOnMetricsChange(
  ScrollMetricsNotification notification, {
  required ScrollPosition? activePosition,
  required Expando<double> previousMaxExtents,
  required bool Function() followsTail,
}) {
  final position = Scrollable.maybeOf(notification.context)?.position;
  if (position == null ||
      !identical(position, activePosition) ||
      !position.hasContentDimensions) {
    return;
  }
  final maxExtent = position.maxScrollExtent;
  final previousMaxExtent = previousMaxExtents[position];
  previousMaxExtents[position] = maxExtent;
  if (previousMaxExtent == null || maxExtent <= previousMaxExtent) return;
  final wasAtEnd =
      position.pixels >= previousMaxExtent - _threadTailScrollTolerance;
  if (!wasAtEnd || !followsTail()) return;
  position.jumpTo(maxExtent);
}

bool _jumpActiveThreadScrollToTail(
  ObjectRef<ScrollPosition?> activePosition,
  bool Function()? testOverride,
) {
  if (testOverride != null) return testOverride();
  final position = activePosition.value;
  if (position == null || !position.hasContentDimensions) return false;
  // Moving the one active viewport avoids a second list and its visible bounce.
  position.jumpTo(position.maxScrollExtent);
  return true;
}

Future<bool> _animateActiveThreadScrollToTail(
  BuildContext context,
  ObjectRef<ScrollPosition?> activePosition,
) async {
  final position = activePosition.value;
  if (position == null || !position.hasContentDimensions) return false;
  if (MediaQuery.disableAnimationsOf(context)) {
    position.jumpTo(position.maxScrollExtent);
    return true;
  }
  await position.animateTo(
    position.maxScrollExtent,
    duration: jumpToLatestScrollDuration,
    curve: jumpToLatestScrollCurve,
  );
  return true;
}

/// Returns whether the thread is at its effective scroll end.
///
/// Item positions can lag or briefly oscillate during lazy layout, so an exact
/// end-of-scroll measurement remains authoritative even while the tail item
/// reports outside the visible boundary.
@visibleForTesting
bool threadTailIsAtEffectiveEnd({
  required bool tailIsLaidOut,
  required bool tailIsVisible,
  required double? extentAfter,
}) =>
    tailIsVisible ||
    (tailIsLaidOut &&
        extentAfter != null &&
        extentAfter <= _threadTailScrollTolerance);

int _threadTailIndex(int replyCount) => replyCount;

void _resumeThreadTailFollow({
  required bool Function() isVisible,
  required ObjectRef<bool> userOptedOut,
  required ObjectRef<bool> followsTail,
}) {
  if (!isVisible()) return;
  userOptedOut.value = false;
  followsTail.value = true;
}

bool _isDeletedBy(Iterable<NostrEvent> events, String messageId) {
  for (final event in events) {
    if (event.kind != EventKind.deletion &&
        event.kind != EventKind.nip29DeleteEvent) {
      continue;
    }
    if (event.tags.any(
      (tag) => tag.length >= 2 && tag[0] == 'e' && tag[1] == messageId,
    )) {
      return true;
    }
  }
  return false;
}

/// Build a lightweight summary for a nested thread (reply that has its own
/// replies). Same logic as the top-level [ThreadSummary] but kept local to
/// avoid coupling.
ThreadSummary _buildNestedSummary(
  String messageId,
  List<TimelineMessage> children,
) {
  final seen = <String>{};
  final participants = <String>[];
  for (var i = children.length - 1; i >= 0 && participants.length < 3; i--) {
    final pk = children[i].pubkey.toLowerCase();
    if (seen.add(pk)) participants.add(pk);
  }
  return ThreadSummary(
    threadHeadId: messageId,
    replyCount: children.length,
    participantPubkeys: participants.reversed.toList(),
    lastReplyAt: children.last.createdAt,
  );
}

/// Serializes deferred tail work behind the latest user scroll intent.
class _ThreadTailIntent {
  var _generation = 0;
  var isDragging = false;

  void detach() => _generation++;

  void beginDrag() {
    isDragging = true;
    detach();
  }

  void endDrag() => isDragging = false;

  void scheduleNextFrame({
    required bool allowed,
    required bool Function() revalidate,
    required VoidCallback action,
  }) {
    if (!allowed) return;
    final generation = ++_generation;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (generation == _generation && revalidate()) action();
    });
  }

  void schedule({
    required bool allowed,
    required bool Function() revalidate,
    required VoidCallback action,
  }) {
    if (!allowed) return;
    final generation = ++_generation;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (generation == _generation && revalidate()) action();
      });
      WidgetsBinding.instance.scheduleFrame();
    });
  }
}

({
  AsyncValue<List<NostrEvent>> relayReplyState,
  List<TimelineMessage> allMsgs,
  TimelineMessage? savedHead,
  Set<String> savedOnlyIds,
  bool accessDenied,
  bool liveDeletionHidesHead,
  bool hasFetchedReplies,
  bool canUseMessagesForInitialTarget,
})
_watchThreadMessages(
  WidgetRef ref,
  ThreadRepliesArgs args,
  TimelineMessage threadHead,
  List<TimelineMessage> routeSnapshot,
  String? currentPubkey, {
  required bool headFromSavedPage,
  required String? initialMessageId,
}) {
  final relayReplyState = ref.watch(threadRepliesProvider(args));
  final replyEvents = ref.watch(threadRepliesWithLocalProvider(args)).value;
  final channelState = ref.watch(channelMessagesProvider(args.channelId));
  final accessDenied = [
    relayReplyState.error,
    channelState.error,
  ].any((error) => error != null && isRelayChannelAccessDenied(error));
  final relayRepliesAvailable = relayReplyState.hasValue;
  final preview = accessDenied || (relayRepliesAvailable && !headFromSavedPage)
      ? null
      : ref.watch(savedThreadPreviewProvider(args.savedPage));
  final saved = preview?.isLoading == false ? preview?.value : null;
  final live = channelState.value ?? const <NostrEvent>[];
  // The relay query carries the aux it observed. Fold in later live changes
  // before formatting so a deletion can suppress a reply from either source.
  final savedReplies = relayRepliesAvailable ? null : saved;
  final events = savedReplies == null && replyEvents == null
      ? live
      : mergeThreadEvents([...?savedReplies, ...?replyEvents], live);
  final observed = accessDenied
      ? const <TimelineMessage>[]
      : formatTimeline(events, currentPubkey: currentPubkey);
  final headDeleted = _isDeletedBy(events, threadHead.id);
  final allMsgs = accessDenied
      ? const <TimelineMessage>[]
      : !relayRepliesAvailable
      ? _provisionalThreadMessages(routeSnapshot, observed, events)
      : [
          if (!headFromSavedPage &&
              !headDeleted &&
              !observed.any((message) => message.id == threadHead.id))
            threadHead,
          ...observed,
        ];
  final hasSavedTarget =
      initialMessageId != null &&
      (saved?.any((event) => event.id == initialMessageId) ?? false) &&
      allMsgs.any((message) => message.id == initialMessageId);
  return (
    relayReplyState: relayReplyState,
    allMsgs: allMsgs,
    // Relay history can replace the saved replies before the channel's
    // separate exact-ID lookup finds a head outside its normal history.
    savedHead:
        relayRepliesAvailable &&
            saved != null &&
            !allMsgs.any((message) => message.id == threadHead.id)
        ? formatTimeline(
            mergeThreadEvents(
              saved.where(
                (event) =>
                    event.id == threadHead.id ||
                    EventKind.channelAuxEventKinds.contains(event.kind),
              ),
              events,
            ),
            currentPubkey: currentPubkey,
          ).where((message) => message.id == threadHead.id).firstOrNull
        : null,
    savedOnlyIds: savedReplies == null
        ? const {}
        : {
            for (final event in savedReplies)
              if (EventKind.channelTimelineContentKinds.contains(event.kind))
                event.id,
          }.difference({
            for (final event in live) event.id,
            for (final event in replyEvents ?? const <NostrEvent>[]) event.id,
            for (final message in routeSnapshot)
              if (!headFromSavedPage || message.id != threadHead.id) message.id,
          }),
    accessDenied: accessDenied,
    liveDeletionHidesHead: headDeleted,
    hasFetchedReplies: replyEvents != null,
    // After a failed fetch, a target in the saved page can be shown during
    // retries. Other targets wait for relay history or a settled fallback.
    canUseMessagesForInitialTarget:
        !accessDenied &&
        (relayRepliesAvailable ||
            (preview?.isLoading != true &&
                relayReplyState.hasError &&
                (!relayReplyState.retrying || hasSavedTarget))),
  );
}

/// Display evidence while the authoritative thread query has no value (still
/// loading, or failed on first load): the route snapshot plus live, cached
/// and optimistic replies. The page keeps the query's loading/error status,
/// so this list is never presented as complete, and nothing is re-queried.
List<TimelineMessage> _provisionalThreadMessages(
  List<TimelineMessage> routeSnapshot,
  List<TimelineMessage> observed,
  List<NostrEvent> liveChannelEvents,
) {
  final byId = <String, TimelineMessage>{
    for (final message in routeSnapshot)
      if (!_isDeletedBy(liveChannelEvents, message.id)) message.id: message,
    for (final message in observed) message.id: message,
  };
  // Same order as compareThreadRepliesChronologically, so replies don't
  // move when the query lands.
  return byId.values.toList()..sort((a, b) {
    final order = a.createdAtMs.compareTo(b.createdAtMs);
    return order != 0 ? order : a.id.compareTo(b.id);
  });
}

/// Resolve the leading visible reply by identity before relay rows reindex
/// the list. This also works when the relay drops the first visible row.
({int index, double alignment})? _threadPreviewAnchor(
  Iterable<ItemPosition> positions,
  List<TimelineMessage> preview,
  List<TimelineMessage> replies,
  double top,
) {
  final indices = {for (final (i, reply) in replies.indexed) reply.id: i + 1};
  final visible =
      positions
          .where(
            (position) =>
                position.index <= preview.length &&
                position.itemLeadingEdge < 1 &&
                position.itemTrailingEdge > top,
          )
          .toList()
        ..sort((a, b) => a.index.compareTo(b.index));
  for (final position in visible) {
    if (position.index == 0) return null;
    final index = indices[preview[position.index - 1].id];
    if (index == null) continue;
    return (index: index, alignment: position.itemLeadingEdge.clamp(0, 1));
  }
  return null;
}
