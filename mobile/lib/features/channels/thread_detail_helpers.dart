part of 'thread_detail_page.dart';

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

/// Thread-scoped composer activity with optional size animation.
class _ThreadTypingIndicator extends StatelessWidget {
  final String channelId;
  final String threadHeadId;
  final bool animated;
  final double horizontalInset;
  final double? overlayTopBoundary;
  final double compactWidthFactor;
  final Animation<double>? composerWidthAnimation;
  final FocusNode? composerFocusNode;
  final ValueNotifier<bool>? composerInteractionLock;
  final ValueNotifier<int>? composerActivationRequests;
  final VoidCallback? onRestoreComposerFocus;

  const _ThreadTypingIndicator({
    required this.channelId,
    required this.threadHeadId,
    this.animated = true,
    this.horizontalInset = Grid.twelve,
    this.overlayTopBoundary,
    this.compactWidthFactor = 1,
    this.composerWidthAnimation,
    this.composerFocusNode,
    this.composerInteractionLock,
    this.composerActivationRequests,
    this.onRestoreComposerFocus,
  });

  @override
  Widget build(BuildContext context) {
    return ComposerAgentActivityIndicator(
      channelId: channelId,
      threadHeadId: threadHeadId,
      animated: animated,
      horizontalInset: horizontalInset,
      overlayTopBoundary: overlayTopBoundary,
      compactWidthFactor: compactWidthFactor,
      composerWidthAnimation: composerWidthAnimation,
      composerFocusNode: composerFocusNode,
      composerInteractionLock: composerInteractionLock,
      composerActivationRequests: composerActivationRequests,
      onRestoreComposerFocus: onRestoreComposerFocus,
    );
  }
}
