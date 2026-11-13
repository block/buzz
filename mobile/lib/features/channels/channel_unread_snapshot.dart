part of 'channels_provider.dart';

/// Keeps the unread catch-up across launches.
///
/// A catch-up cursor only says the relay was already read up to a point; the
/// unread events found before that point are what keep a channel unread. Both
/// are saved together after a catch-up settles and restored together when the
/// scope is built, so a cold start asks only for what is new, the same as a
/// later catch-up in one session. A channel saved without its cursor, or state
/// that could not be saved, gets the full window again.
extension _UnreadSnapshot on ChannelsNotifier {
  /// Restores [pubkey]'s saved unread state on [relay] and returns the writer
  /// for it, or null when there is no identity or storage.
  void Function()? _captureUnreadSnapshot(
    ChannelListSnapshotStorage storage,
    ChannelListSnapshotToken token,
    String pubkey,
    String relay,
  ) {
    final saved = storage.readUnread(pubkey, relay);
    if (saved != null) {
      try {
        _restoreUnread(saved);
      } catch (_) {
        _unreadCaughtUpByChannel.clear();
        _observedUnreadEventsByChannel.clear();
        _latestObservedByChannel.clear();
      }
    }
    return () {
      final unread = _encodeUnread();
      unawaited(
        storage
            .writeUnread(token, unread)
            .then(
              (saved) {
                if (!saved) debugPrint('[ChannelsNotifier] unread not saved');
              },
              onError: (Object error, StackTrace stack) {
                debugPrint('[ChannelsNotifier] unread write failed: $error');
              },
            ),
      );
    };
  }

  Set<String> _pendingThreadReplyIds(String channelId) => {
    for (final event
        in _observedUnreadEventsByChannel[channelId]?.values ??
            const <ObservedUnreadEvent>[])
      if (event.threadReplyPending) event.id,
  };

  /// Forgets [ids] that are still waiting for their payload. Returns whether
  /// any were.
  bool _dropPendingThreadReplies(String channelId, Set<String> ids) {
    final events = _observedUnreadEventsByChannel[channelId];
    if (events == null) return false;
    final before = events.length;
    events.removeWhere(
      (id, event) => event.threadReplyPending && ids.contains(id),
    );
    if (events.length == before) return false;
    if (events.isEmpty) {
      _observedUnreadEventsByChannel.remove(channelId);
      _latestObservedByChannel.remove(channelId);
    } else {
      _latestObservedByChannel[channelId] = events.values
          .map((event) => event.createdAt)
          .reduce(max);
    }
    return true;
  }

  void _restoreUnread(Map<String, dynamic> saved) {
    final channels = saved['channels'] as Map<String, dynamic>;
    for (final MapEntry(key: channelId, value: raw) in channels.entries) {
      final json = raw as Map<String, dynamic>;
      _unreadCaughtUpByChannel[channelId] = (
        sentAt: json['sentAt'] as int,
        readAt: json['readAt'] as int?,
        muted: json['muted'] as bool,
      );
      final events = [
        for (final event in json['events'] as List<dynamic>)
          _decodeObserved(event as List<dynamic>),
      ];
      if (events.isEmpty) continue;
      _observedUnreadEventsByChannel[channelId] = {
        for (final event in events) event.id: event,
      };
      _latestObservedByChannel[channelId] = events
          .map((event) => event.createdAt)
          .reduce(max);
    }
  }

  /// Every caught-up channel with the unread events its read timestamp has
  /// not covered yet. A channel without a cursor is left out: its events
  /// alone cannot say how far the relay was read.
  Map<String, dynamic> _encodeUnread() {
    final readState = _lifecycleRef.read(readStateProvider);
    return {
      'channels': {
        for (final MapEntry(key: channelId, value: cursor)
            in _unreadCaughtUpByChannel.entries)
          channelId: {
            'sentAt': cursor.sentAt,
            'readAt': cursor.readAt,
            'muted': cursor.muted,
            'events': [
              for (final event
                  in _observedUnreadEventsByChannel[channelId]?.values ??
                      const <ObservedUnreadEvent>[])
                if (event.createdAt >
                    (readState.effectiveTimestamp(channelId) ?? -1))
                  _encodeObserved(event),
            ],
          },
      },
    };
  }
}

// Saved compactly, as `[id, createdAt, flags, rootId?]`. A thread reply's
// payload is left out: it dominates the size (about 1 KB each), so the next
// catch-up fetches it by ID instead.
const _highPriority = 1, _badge = 2, _appBadge = 4, _channelCatchUp = 8;
const _threadReply = 16;

List<Object?> _encodeObserved(ObservedUnreadEvent event) => [
  event.id,
  event.createdAt,
  (event.highPriority ? _highPriority : 0) |
      (event.countsTowardBadge ? _badge : 0) |
      (event.countsTowardAppBadge ? _appBadge : 0) |
      (event.channelCatchUp ? _channelCatchUp : 0) |
      (event.threadReply != null || event.threadReplyPending
          ? _threadReply
          : 0),
  ?event.rootId,
];

ObservedUnreadEvent _decodeObserved(List<dynamic> json) {
  final flags = json[2] as int;
  return ObservedUnreadEvent(
    id: json[0] as String,
    createdAt: json[1] as int,
    rootId: json.length > 3 ? json[3] as String : null,
    highPriority: flags & _highPriority != 0,
    countsTowardBadge: flags & _badge != 0,
    countsTowardAppBadge: flags & _appBadge != 0,
    channelCatchUp: flags & _channelCatchUp != 0,
    threadReplyPending: flags & _threadReply != 0,
  );
}
