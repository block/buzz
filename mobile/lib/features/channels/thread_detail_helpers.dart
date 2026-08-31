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

/// Thread-scoped typing status with optional size animation.
class _ThreadTypingIndicator extends StatelessWidget {
  final List<TypingEntry> entries;
  final bool animated;

  const _ThreadTypingIndicator({required this.entries, this.animated = true});

  @override
  Widget build(BuildContext context) {
    final child = entries.isEmpty
        ? const SizedBox.shrink()
        : ChannelTypingIndicator(entries: entries);
    if (!animated || MediaQuery.disableAnimationsOf(context)) return child;
    return AnimatedSize(
      duration: const Duration(milliseconds: 180),
      curve: Curves.easeOutCubic,
      alignment: Alignment.bottomCenter,
      child: child,
    );
  }
}
