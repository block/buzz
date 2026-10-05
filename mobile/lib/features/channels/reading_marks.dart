import 'dart:async';

import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import 'timeline_message.dart';
import 'unread_badge/is_high_priority_event.dart';
import 'unread_badge/observed_unread_event.dart';

/// How long a row must stay fully visible before it counts as read. This is
/// the same dwell the web and desktop app (`buzz-app`) uses.
const readingDwell = Duration(milliseconds: 300);

/// The read marks to write after a reading dwell, following `buzz-app`'s
/// rules (its `docs/unread.md`):
///
/// - When the newest message is at the bottom, a channel writes
///   `activity:<channel>` and a thread writes `thread-activity:<root>`. These
///   catch-up marks read ordinary messages and that thread's replies. They do
///   not read mentions, broadcasts or DMs.
/// - Each fully visible message that no mark reads yet gets its own `msg:`
///   mark. Messages the catch-up mark reads do not need one.
/// - Automatic reading never writes the channel mark. Only an explicit
///   Mark read does that.
///
/// [loaded] is every message loaded for the channel or thread, including
/// replies. [visible] is the messages fully visible for the whole dwell.
/// [bottom] is the newest message when its bottom edge is visible at the
/// bottom of the list; otherwise null. Set [threadRootId] when reading a
/// thread. The result maps each read-state context to its new time.
Map<String, int> readingMarks({
  required ReadStateState readState,
  required String channelId,
  required bool isDm,
  required String? currentPubkey,
  required Iterable<TimelineMessage> loaded,
  required Iterable<TimelineMessage> visible,
  TimelineMessage? bottom,
  String? threadRootId,
}) {
  final marks = <String, int>{};
  int? markerOf(String contextId) =>
      maxReadAt([marks[contextId], readState.effectiveTimestamp(contextId)]);

  if (bottom != null) {
    final catchUp = _catchUpMark(
      channelId: channelId,
      loaded: loaded,
      bottom: bottom,
      threadRootId: threadRootId,
      markerOf: markerOf,
    );
    if (catchUp != null) marks[catchUp.key] = catchUp.value;
  }

  final self = currentPubkey?.toLowerCase();
  for (final message in visible) {
    if (message.isSystem || message.pubkey.toLowerCase() == self) continue;
    final contextId = msgContextKey(message.id);
    // Automatic reading keeps a message the reader marked unread.
    if (readState.isForcedUnread(contextId)) continue;
    final readAt = observedUnreadEventReadAt(
      _observed(message, isDm: isDm, currentPubkey: self ?? ''),
      channelId,
      markerOf,
    );
    if (readAt != null && readAt >= message.createdAt) continue;
    marks[contextId] = message.createdAt;
  }
  return marks;
}

/// The catch-up mark for [bottom], or null when it would read nothing new.
MapEntry<String, int>? _catchUpMark({
  required String channelId,
  required Iterable<TimelineMessage> loaded,
  required TimelineMessage bottom,
  required String? threadRootId,
  required int? Function(String contextId) markerOf,
}) {
  // A loaded message newer than the bottom row means the list does not show
  // the live bottom, for example after a jump into history.
  final showsNewest = !loaded.any(
    (message) =>
        message.createdAt > bottom.createdAt &&
        (threadRootId != null
            ? _threadRootOf(message) == threadRootId
            : message.parentId == null || _isBroadcast(message)),
  );
  if (!showsNewest) return null;

  // Replies can be newer than the newest top-level message. `activity:`
  // never reads replies, so a time that includes them is safe. `buzz-app`
  // does the same.
  var cut = bottom.createdAt;
  if (threadRootId == null) {
    for (final message in loaded) {
      if (message.createdAt > cut) cut = message.createdAt;
    }
  }
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

/// Describes [message] the way the unread badge does, so this policy and
/// the unread display agree on which marks read it.
ObservedUnreadEvent _observed(
  TimelineMessage message, {
  required bool isDm,
  required String currentPubkey,
}) {
  final broadcast = _isBroadcast(message);
  return makeObservedUnreadEvent(
    id: message.id,
    createdAt: message.createdAt,
    rootId: broadcast ? null : _threadRootOf(message),
    highPriority: isDm || isHighPriorityEvent(message.tags, currentPubkey),
    channelType: isDm ? 'dm' : null,
    isThreadedReply: message.parentId != null && !broadcast,
  );
}

String? _threadRootOf(TimelineMessage message) =>
    message.parentId == null ? null : message.rootId ?? message.parentId;

bool _isBroadcast(TimelineMessage message) => message.tags.any(
  (tag) => tag.length >= 2 && tag[0] == 'broadcast' && tag[1] == '1',
);

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
