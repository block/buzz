part of '../thread_detail_page.dart';

/// Computes where a re-opened thread should land and which reply carries the
/// unread divider.
///
/// `firstUnreadReplyId` is the oldest reply the reader has not seen, or `null`
/// when the thread is fully read. `resumeReplyIndex` is that reply's index in
/// [replies], or `-1` when an ordinary tail settle should stand.
///
/// Must be called from `build`, unconditionally and in a stable position: it
/// owns a `useRef` snapshot.
({String? firstUnreadReplyId, int resumeReplyIndex}) _useThreadResumeTarget({
  required ReadStateState readState,
  required List<TimelineMessage> replies,
  required String channelId,
  required String queryRootId,
  required String? currentPubkey,
}) {
  // Freeze each reply's effective read timestamp the first time we see it.
  // This has to happen during build, before the caller's post-frame effect
  // marks everything read: markers are monotonic, so reading them after
  // that pass would report the whole thread as already seen and there
  // would be nothing left to resume to.
  final openReadSnapshot = useRef(<String, int?>{});
  if (readState.isReady) {
    for (final reply in replies) {
      openReadSnapshot.value.putIfAbsent(
        reply.id,
        () => effectiveMessageReadAt(
          readState,
          channelId: channelId,
          messageId: reply.id,
          threadRootId: queryRootId,
        ),
      );
    }
  }

  final firstUnreadReplyId = readState.isReady
      ? firstUnreadThreadReplyId(
          replies: replies,
          openReadSnapshot: openReadSnapshot.value,
          isForcedUnread: (messageId) =>
              readState.isForcedUnread(msgContextKey(messageId)),
          currentPubkey: currentPubkey,
        )
      : null;

  // Resuming is for *returning* to a thread. A reader with no marker over
  // any reply — never opened this thread, never read the channel — has no
  // place to return to, so an ordinary open still settles on the tail the
  // way it always has. The divider still marks the replies as new; it just
  // does not drag the reader to the top of a thread they have never seen.
  // Once the channel marker alone covers the replies this turns back on,
  // which is the case the resume exists for.
  final hasThreadReadHistory = openReadSnapshot.value.values.any(
    (readAt) => readAt != null,
  );
  final resumeReplyIndex = hasThreadReadHistory && firstUnreadReplyId != null
      ? replies.indexWhere((reply) => reply.id == firstUnreadReplyId)
      : -1;

  return (
    firstUnreadReplyId: firstUnreadReplyId,
    resumeReplyIndex: resumeReplyIndex,
  );
}
