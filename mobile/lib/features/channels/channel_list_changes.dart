part of 'channels_provider.dart';

/// Relay-signed notices that tag this reader when its channel list changes:
/// it was added to a channel (including a new DM) or removed from one, or one
/// of its DMs was hidden or shown on another device.
const _channelListNoticeKinds = [
  EventKind.memberAddedNotification,
  EventKind.memberRemovedNotification,
  EventKind.dmVisibility,
];

/// System notices (kind:40099) on a listed channel that change the channel
/// itself rather than its messages or members.
const _channelChangeSystemTypes = {
  'auto_join_changed',
  'channel_archived',
  'channel_deleted',
  'channel_unarchived',
  'join_role_changed',
  'purpose_changed',
  'topic_changed',
  'ttl_changed',
  'unlisted_changed',
  'visibility_changed',
};

/// Tracks whether the channel list may have changed since it was last read.
///
/// The list is read on every launch, resume, reconnect and pull to refresh.
/// Between those, live events say when it changed: a notice for this reader,
/// or a metadata edit or system notice on a listed channel. The backstop timer
/// reads the list again only after one of them, instead of every minute.
extension _ChannelListChanges on ChannelsNotifier {
  /// Notes a live event on a listed channel that changes the channel itself.
  /// Returns whether it was one.
  bool _noteChannelChange(NostrEvent event) {
    final changed =
        event.kind == EventKind.nip29EditMetadata ||
        (event.kind == EventKind.systemMessage &&
            _channelChangeSystemTypes.contains(_systemMessageType(event)));
    if (changed) _channelListChanges++;
    return changed;
  }

  /// Subscribes to [_channelListNoticeKinds] once per relay and identity, and
  /// again after a new connection if the relay refused it.
  Future<void> _subscribeChannelListNotices(String myPk) async {
    if (_listNoticesSubscribed || _listNoticesRefused) return;
    _listNoticesSubscribed = true;
    final generation = ++_listNoticesGeneration;
    try {
      final unsubscribe = await _lifecycleRef
          .read(relaySessionProvider.notifier)
          .subscribe(
            NostrFilter(
              kinds: _channelListNoticeKinds,
              tags: {
                '#p': [myPk],
              },
              // Overlap the list read that came just before, so a notice sent
              // while it was out still arrives. A replayed older notice only
              // costs one extra read.
              since:
                  DateTime.now().millisecondsSinceEpoch ~/ 1000 -
                  relayLiveWindowSkewSeconds,
              limit: 0,
            ),
            (_) {
              if (generation == _listNoticesGeneration) _channelListChanges++;
            },
            onClosed: (message) {
              if (generation != _listNoticesGeneration) return;
              // Terminal: a new connection may admit it again.
              _listNoticesUnsubscribe = null;
              _listNoticesSubscribed = false;
              _listNoticesRefused = true;
              debugPrint(
                '[ChannelsNotifier] channel list notices closed: $message',
              );
            },
          );
      if (generation != _listNoticesGeneration) {
        unsubscribe();
        return;
      }
      _listNoticesUnsubscribe = unsubscribe;
    } catch (error) {
      if (generation != _listNoticesGeneration) return;
      _listNoticesSubscribed = false;
      debugPrint('[ChannelsNotifier] channel list notices failed: $error');
    }
  }

  void _clearChannelListNotices() {
    _listNoticesGeneration++;
    _listNoticesUnsubscribe?.call();
    _listNoticesUnsubscribe = null;
    _listNoticesSubscribed = false;
  }
}

String? _systemMessageType(NostrEvent event) {
  try {
    final content = jsonDecode(event.content);
    return content is Map ? content['type'] as String? : null;
  } on FormatException {
    return null;
  } on TypeError {
    return null;
  }
}
