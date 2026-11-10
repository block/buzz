import 'huddle_timeline.dart';
import 'dart:async';

import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:scrollable_positioned_list/scrollable_positioned_list.dart';

import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import 'timeline_message.dart';
import 'unread_badge/is_high_priority_event.dart';

/// How long a row must stay readable ([isReadableRow]) before it counts as
/// read. This is the same dwell the web and desktop app (`buzz-app`) uses.
const readingDwell = Duration(milliseconds: 300);

/// The read marks to write after a reading dwell, following `buzz-app`'s
/// rules (its `docs/unread.md`):
///
/// - When the newest message is at the bottom, a channel writes
///   `activity:<channel>` and a thread writes `thread-activity:<root>`. These
///   catch-up marks read ordinary messages and that thread's replies. They do
///   not read mentions, broadcasts or DMs.
/// - Unlike `buzz-app`, the newest message itself is read at the live bottom,
///   in a thread too, so one taller than the screen is read at its end.
///   Mentions above it are read only on their own, never by reaching the
///   bottom.
/// - Each readable message (see [isReadableRow]) gets its own `msg:` mark
///   unless its own mark, the channel mark, or (for an ordinary top-level
///   message) `activity:` already reads it. A thread mark never counts here: a reply finds its
///   thread only while the root is loaded, so `buzz-app` keeps reply marks.
///   Unlike `buzz-app`, the message a link opened is readable once any part
///   of it shows, since a phone often cannot fit a long one.
/// - Automatic reading never writes the channel mark. Only an explicit
///   Mark read does that.
///
/// [loaded] is every message loaded for the channel or thread, including
/// replies. [visible] is the messages readable for the whole dwell; see
/// [isReadableRow].
/// [bottom] is the newest message when its bottom edge is visible at the
/// bottom of the list; otherwise null. Set [threadRootId] when reading a
/// thread, and [isRootThread] when its head is the thread root rather than a
/// nested reply. Set [flat] for a channel timeline that shows replies inline,
/// as a Solo chat does, so its newest reply is the bottom. [now] is the current Unix time in seconds; it defaults to
/// the clock. The result maps each read-state context to its new time.
Map<String, int> readingMarks({
  required ReadStateState readState,
  required String channelId,
  required bool isDm,
  required String? currentPubkey,
  required Iterable<TimelineMessage> loaded,
  required Iterable<TimelineMessage> visible,
  TimelineMessage? bottom,
  String? threadRootId,
  bool isRootThread = true,
  bool flat = false,
  int? now,
}) {
  final marks = <String, int>{};
  int? markerOf(String contextId) =>
      maxReadAt([marks[contextId], readState.effectiveTimestamp(contextId)]);

  final self = currentPubkey?.toLowerCase();
  bool needsMark(TimelineMessage message) {
    if (message.isSystem || message.pubkey.toLowerCase() == self) return false;
    final contextId = msgContextKey(message.id);
    // Automatic reading keeps a message the reader marked unread.
    if (readState.isForcedUnread(contextId)) return false;
    final ordinary = readByChannelCatchUp(
      isDm: isDm,
      isReply: message.parentId != null && !_isBroadcast(message),
      highPriority: self == null || isHighPriorityEvent(message.tags, self),
    );
    final readAt = maxReadAt([
      markerOf(contextId),
      markerOf(channelId),
      if (ordinary) markerOf(activityContextKey(channelId)),
    ]);
    return readAt == null || readAt < message.createdAt;
  }

  void markMessage(TimelineMessage message) {
    if (needsMark(message)) {
      marks[msgContextKey(message.id)] = message.createdAt;
    }
  }

  final liveBottom =
      bottom != null &&
          _showsNewest(
            loaded: loaded,
            bottom: bottom,
            threadRootId: threadRootId,
            flat: flat,
          )
      ? bottom
      : null;
  // A nested thread view shows only one branch, so reaching its bottom does
  // not read the whole thread.
  if (liveBottom != null && (threadRootId == null || isRootThread)) {
    final catchUp = _catchUpMark(
      channelId: channelId,
      bottom: liveBottom,
      threadRootId: threadRootId,
      markerOf: markerOf,
      now: now ?? DateTime.now().millisecondsSinceEpoch ~/ 1000,
    );
    if (catchUp != null) marks[catchUp.key] = catchUp.value;
  }
  // The newest row is read once its end shows at the live bottom, even when
  // it is taller than the screen and never fills it, as at the end of a thread.
  if (liveBottom != null) markMessage(liveBottom);

  visible.forEach(markMessage);
  return marks;
}

/// Whether a row at [position] counts as read: it lies inside the reading
/// area from [start] to [end], or it is taller than that area and covers all
/// of it. A [linked] row, the message a link opened, counts once any part of
/// it is inside: the jump can leave a long one cut off.
bool isReadableRow(
  ItemPosition position, {
  required double start,
  required double end,
  bool linked = false,
}) => linked
    ? position.itemLeadingEdge < end && position.itemTrailingEdge > start
    : (position.itemLeadingEdge >= start && position.itemTrailingEdge <= end) ||
          (position.itemLeadingEdge <= start &&
              position.itemTrailingEdge >= end);

/// Whether [bottom] is the live bottom. A loaded message newer than the
/// bottom row means the list does not show it, for example after a jump
/// into history.
bool _showsNewest({
  required Iterable<TimelineMessage> loaded,
  required TimelineMessage bottom,
  required String? threadRootId,
  required bool flat,
}) => !loaded.any(
  (message) =>
      (threadRootId != null || !isCollapsedHuddleEnding(message, loaded)) &&
      // The main timeline never renders these, so they can't be its bottom.
      !isHiddenMembershipNotice(message) &&
      message.createdAt > bottom.createdAt &&
      (threadRootId != null
          ? _threadRootOf(message) == threadRootId
          : flat || message.parentId == null || _isBroadcast(message)),
);

/// The catch-up mark for [bottom], or null when it would read nothing new.
MapEntry<String, int>? _catchUpMark({
  required String channelId,
  required TimelineMessage bottom,
  required String? threadRootId,
  required int? Function(String contextId) markerOf,
  required int now,
}) {
  // Never read beyond the last rendered message: a message between that
  // row and a hidden lifecycle event may still arrive out of order.
  final cut = bottom.createdAt < now ? bottom.createdAt : now;
  final key = threadRootId != null
      ? threadActivityContextKey(threadRootId)
      : activityContextKey(channelId);
  final covered = maxReadAt([
    markerOf(key),
    markerOf(channelId),
    if (threadRootId != null) markerOf(threadContextKey(threadRootId)),
  ]);
  if (covered != null && covered >= cut) return null;
  return MapEntry(key, cut);
}

String? _threadRootOf(TimelineMessage message) =>
    message.parentId == null ? null : message.rootId ?? message.parentId;

bool _isBroadcast(TimelineMessage message) => message.tags.any(
  (tag) => tag.length >= 2 && tag[0] == 'broadcast' && tag[1] == '1',
);

/// A stable dwell key for [messages]. Pages rebuild a new message list on
/// every frame that touches them, such as a typing indicator, so the list
/// itself would restart the dwell each time. This changes only when a
/// message arrives, leaves, or moves the newest time.
(int, int, String?) readingContentKey(Iterable<TimelineMessage> messages) {
  var count = 0;
  TimelineMessage? newest;
  for (final message in messages) {
    count++;
    if (newest == null || message.createdAt > newest.createdAt) {
      newest = message;
    }
  }
  return (count, newest?.createdAt ?? 0, newest?.id);
}

/// Whether the app is in the foreground. Null means the platform has not
/// reported a state yet, which only happens before the first frame and in
/// tests.
bool isAppInUse(AppLifecycleState? state) =>
    state == null || state == AppLifecycleState.resumed;

/// Runs [onDwell] once the list's item positions have stayed still for
/// [readingDwell]. Any position change restarts the wait. Changing [keys]
/// restarts it too, for changes that move no rows, such as new content or
/// the route coming back to the front. The wait is cancelled while
/// [active] is false. [onDwell] reads the current state when it runs.
void useReadingDwell({
  required ValueListenable<Object?> positions,
  required bool active,
  required VoidCallback onDwell,
  List<Object?> keys = const [],
}) {
  final callback = useRef(onDwell);
  callback.value = onDwell;
  useEffect(() {
    if (!active) return null;
    Timer? timer;
    void restart() {
      timer?.cancel();
      timer = Timer(readingDwell, () {
        timer = null;
        callback.value();
      });
    }

    positions.addListener(restart);
    restart();
    return () {
      timer?.cancel();
      positions.removeListener(restart);
    };
  }, [positions, active, ...keys]);
}
