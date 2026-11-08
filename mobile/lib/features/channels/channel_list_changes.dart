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
  'channel_auto_archived',
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
/// or a metadata edit or system notice on a listed channel. Each one schedules
/// a read (see [_ChannelsNotifierLiveSubscriptions._noteListChanged]).
///
/// The live subscriptions ask for no stored events (`limit: 0`), so nothing
/// sent while one is down is replayed. Each list read therefore opens the
/// notice subscription before it reads, and reopening a subscription between
/// reads schedules a read to catch up on what it missed.
extension _ChannelListChanges on ChannelsNotifier {
  /// Notes a live event on a listed channel that changes the channel itself.
  /// Returns whether it was one.
  bool _noteChannelChange(NostrEvent event) {
    final changed =
        event.kind == EventKind.nip29EditMetadata ||
        (event.kind == EventKind.systemMessage &&
            _channelChangeSystemTypes.contains(_systemMessageType(event)));
    if (changed) _noteListChanged();
    return changed;
  }

  /// Subscribes to [_channelListNoticeKinds] unless already subscribed. The
  /// notifier rebuilds when the relay or identity changes, so one subscription
  /// serves its whole lifetime. A refused or failed subscription is retried
  /// with backoff. Notices sent while it was down are not replayed, so
  /// reopening it ([isRetry]) schedules a read (see
  /// [_ChannelsNotifierLiveSubscriptions._noteReopenedSubscriptions]).
  Future<void> _subscribeChannelListNotices(
    String myPk, {
    bool isRetry = false,
  }) async {
    if (_listNoticesSubscribed) return;
    _listNoticesSubscribed = true;
    _listNoticesRetry.cancelPending();
    final generation = ++_listNoticesGeneration;
    var closed = false;
    try {
      final unsubscribe = await _lifecycleRef
          .read(relaySessionProvider.notifier)
          .subscribeWithStatus(
            NostrFilter(
              kinds: _channelListNoticeKinds,
              tags: {
                '#p': [myPk],
              },
              limit: 0,
            ),
            (_) {
              if (generation == _listNoticesGeneration) _noteListChanged();
            },
            onClosed: (message) {
              if (generation != _listNoticesGeneration) return;
              closed = true;
              _listNoticesUnsubscribe = null;
              _listNoticesInterrupted = false;
              debugPrint(
                '[ChannelsNotifier] channel list notices closed: $message',
              );
              if (_isRelayCapacityClose(message)) {
                _retryChannelListNotices(myPk);
              } else {
                _listNoticesRefused = true;
              }
            },
            // RelaySession reopens a REQ the relay closed for a transient
            // reason itself, and reports it only here.
            onStatusChanged: (status) {
              if (generation != _listNoticesGeneration) return;
              if (status == RelaySubscriptionStatus.retrying) {
                _listNoticesInterrupted = true;
              } else if (_listNoticesInterrupted) {
                _listNoticesInterrupted = false;
                _listNoticesReopened = true;
                _scheduleListRead();
              }
            },
          );
      if (generation != _listNoticesGeneration || closed) {
        unsubscribe();
        return;
      }
      _listNoticesUnsubscribe = unsubscribe;
      if (isRetry) {
        _listNoticesReopened = true;
        _listNoticesRetry.opened();
        _scheduleListRead();
      }
    } catch (error) {
      // A refusal before readiness also throws; onClosed has handled it.
      if (generation != _listNoticesGeneration || _listNoticesRefused) return;
      debugPrint('[ChannelsNotifier] channel list notices failed: $error');
      _retryChannelListNotices(myPk);
    }
  }

  void _retryChannelListNotices(String myPk) {
    _listNoticesSubscribed = false;
    _listNoticesRetry.schedule(() {
      // Disconnected: the reconnect's list read subscribes again.
      if (!_lifecycleRef.mounted ||
          _lifecycleRef.read(relaySessionProvider).status !=
              SessionStatus.connected) {
        return;
      }
      unawaited(_subscribeChannelListNotices(myPk, isRetry: true));
    });
  }

  void _clearChannelListNotices() {
    _listNoticesGeneration++;
    _listNoticesRetry.cancel();
    _listNoticesReopened = false;
    _listNoticesInterrupted = false;
    _listNoticesRefused = false;
    _listNoticesUnsubscribe?.call();
    _listNoticesUnsubscribe = null;
    _listNoticesSubscribed = false;
  }
}

/// Whether the relay closed a REQ for lack of capacity, which can clear on
/// its own. Its other terminal reasons (`restricted:`, `invalid:` and so on)
/// cannot succeed unchanged on this connection, so those REQs are not retried.
bool _isRelayCapacityClose(String message) =>
    message.trim().toLowerCase().startsWith('error: too many subscriptions');

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
