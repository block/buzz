import 'timeline_message.dart';

/// Oldest reply the reader has not seen, or `null` when the thread is fully
/// read. Mirrors desktop's `computeThreadUnreadMarker`.
///
/// [openReadSnapshot] is the per-reply *effective* read timestamp frozen when
/// the thread was opened, keyed by reply id — callers must fold the channel
/// and thread markers in via `effectiveMessageReadAt` rather than reading a
/// bare `msg:` marker, or a thread whose replies were only ever covered by the
/// channel marker reads as entirely unread. A reply is unread when forced,
/// when its snapshot value is `null` (never read), or when it is newer than
/// that value. Replies missing from the snapshot are skipped: they were never
/// observed at open time, so there is nothing to compare against.
///
/// A captured `null` is a real answer ("never read"), so callers must build the
/// snapshot with `putIfAbsent` and this function must test membership with
/// `containsKey` rather than collapsing `null` with `??` — otherwise a
/// never-read reply is indistinguishable from an unobserved one and the marker
/// silently walks past it.
///
/// Replies authored by [currentPubkey] never count as unread; the reader
/// already knows about their own posts.
String? firstUnreadThreadReplyId({
  required List<TimelineMessage> replies,
  required Map<String, int?> openReadSnapshot,
  required bool Function(String messageId) isForcedUnread,
  String? currentPubkey,
}) {
  final localPubkey = currentPubkey?.toLowerCase();
  for (final reply in replies) {
    if (localPubkey != null && reply.pubkey.toLowerCase() == localPubkey) {
      continue;
    }
    if (!openReadSnapshot.containsKey(reply.id)) continue;
    final readAt = openReadSnapshot[reply.id];
    if (isForcedUnread(reply.id) ||
        readAt == null ||
        reply.createdAt > readAt) {
      return reply.id;
    }
  }
  return null;
}
