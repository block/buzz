part of 'channels_provider.dart';

extension _UnreadCatchUp on ChannelsNotifier {
  /// Backfills unread badges for the channels this refresh just installed.
  ///
  /// Runs detached from the refresh that starts it, so the lifecycle token
  /// captured below is what keeps a response that outlived its refresh from
  /// writing unread state into whatever the user is looking at now. Every
  /// refresh path starts a catch-up, including the initial load, the ordinary
  /// membership refresh a join performs and the reconnect backstop, so the
  /// token is unconditional rather than tied to discovery. A retired refresh
  /// returns instead of throwing: nothing awaits this future, so a thrown
  /// [_StaleChannelRefresh] would only surface as an unhandled error.
  ///
  /// The request fence and subscription generation are passed from the refresh
  /// that installed [channels], preserving request ownership after detachment.
  Future<void> _catchUpUnreadEvents(
    List<Channel> channels,
    _ChannelRefreshFence fence,
    int subscriptionGeneration,
  ) async {
    if (!_lifecycleRef.mounted) return;
    try {
      final myPk = _lifecycleRef.read(myPubkeyProvider);
      if (myPk == null) return;

      final session = _lifecycleRef.read(relaySessionProvider.notifier);
      final mutedChannelIds = _mutedChannelIds();
      final readState = _lifecycleRef.read(readStateProvider);
      final activeChannels = [
        for (final channel in channels)
          if (channel.isMember && !channel.isArchived) channel,
      ];
      final channelById = {
        for (final channel in activeChannels) channel.id: channel,
      };
      final readAtByChannel = {
        for (final channel in activeChannels)
          channel.id: readState.effectiveTimestamp(channel.id),
      };
      final requestedAt = currentUnixSeconds();
      final filters = [
        for (final channel in activeChannels)
          for (final kinds in [
            EventKind.channelMessageEventKinds,
            [EventKind.deletion, EventKind.nip29DeleteEvent],
          ])
            NostrFilter(
              kinds: kinds,
              tags: {
                '#h': [channel.id],
              },
              since: _unreadCatchUpSince(
                channel.id,
                readAtByChannel[channel.id],
              ),
              limit: _unreadCatchUpLimit,
            ),
      ];

      final failedChannelIds = <String>{};
      final events = await _fetchChannelHistoryBatch(
        session,
        filters,
        operation: 'unread catch-up',
        failedChannelIds: failedChannelIds,
      );
      // Unavailable: keep the unread state already observed.
      if (events == null) return;
      // The relay round-trip above is the window Jed's probes park in: a newer
      // refresh, a community switch or an identity switch here means every
      // write below belongs to a channel list the user has left.
      if (!_lifecycleRef.mounted ||
          _isCatchUpRetired(fence, subscriptionGeneration)) {
        return;
      }

      _recordUnreadCaughtUp(readAtByChannel, requestedAt, failedChannelIds);

      // Keep this response's deletion evidence independently of the bounded
      // cross-request tombstones: a large batch can evict its own early IDs.
      final batchDeletedIds = <String>{};
      var appliedDeletion = false;
      for (final event in events) {
        final channelId = event.channelId;
        if (channelById.containsKey(channelId) ||
            (channelId == null && event.kind == EventKind.deletion)) {
          if (_applyUnreadDeletion(event)) {
            appliedDeletion = true;
            batchDeletedIds.addAll(
              event.tags
                  .where((tag) => tag.length >= 2 && tag[0] == 'e')
                  .map((tag) => tag[1]),
            );
          }
        }
      }

      for (final event in events) {
        if (batchDeletedIds.contains(event.id)) continue;
        if (EventKind.channelMessageEventKinds.contains(event.kind) &&
            event.pubkey.toLowerCase() == myPk.toLowerCase()) {
          _recordSelfThreadInterest(event, myPk);
        }
      }

      final recordedAtByChannel = <String, int>{};
      final windowSizeByChannel = <String, int>{};
      final oldestQuietByChannel = <String, NostrEvent>{};
      for (final event in events) {
        if (!EventKind.channelMessageEventKinds.contains(event.kind)) continue;
        final channelId = event.channelId;
        if (channelId == null) continue;
        final channel = channelById[channelId];
        if (channel == null) continue;
        windowSizeByChannel.update(channelId, (n) => n + 1, ifAbsent: () => 1);
        if (batchDeletedIds.contains(event.id) ||
            _unscopedUnreadDeletedIds.contains(event.id) ||
            _unreadDeletedIdsByChannel[channelId]?.contains(event.id) == true) {
          continue;
        }
        final readAt = readAtByChannel[channelId];
        if (event.pubkey.toLowerCase() == myPk.toLowerCase()) continue;
        if (readAt != null && event.createdAt <= readAt) continue;
        if (!shouldNotifyForEvent(
          event,
          myPk,
          participatedRootIds: _participatedRootIds,
          followedRootIds: _followedRootIds(),
          authoredRootIds: _authoredRootIds,
          mutedChannelIds: mutedChannelIds,
          channelId: channel.id,
        )) {
          continue;
        }
        if (isQuietAgentMessage(event.tags)) {
          final oldest = oldestQuietByChannel[channelId];
          if (oldest == null || event.createdAt < oldest.createdAt) {
            oldestQuietByChannel[channelId] = event;
          }
          continue;
        }
        if (!_recordUnreadEvent(channel, event, myPk)) continue;
        recordedAtByChannel[channelId] = max(
          recordedAtByChannel[channelId] ?? 0,
          event.createdAt,
        );
      }
      // A full window of quiet agent messages can hide an older unread event,
      // and every refresh asks for the same window. Count its oldest quiet
      // message instead, so the channel stays unread until it is opened.
      for (final MapEntry(key: channelId, value: quiet)
          in oldestQuietByChannel.entries) {
        if (recordedAtByChannel.containsKey(channelId)) continue;
        if ((windowSizeByChannel[channelId] ?? 0) < _unreadCatchUpLimit) {
          continue;
        }
        if (_recordUnreadEvent(
          channelById[channelId]!,
          quiet,
          myPk,
          countQuiet: true,
        )) {
          recordedAtByChannel[channelId] = quiet.createdAt;
        }
      }
      // Republish only when this catch-up actually changed unread state. A
      // batch that recorded nothing has nothing to show, and a failed or
      // superseded batch must not repaint another refresh's list: the retired
      // check above already returned in that case, and no await separates it
      // from here, so a second check would be dead code.
      // Recorded events also advance `lastMessageAt`, as live events do: a
      // deadline-unavailable latest-message batch leaves it unset.
      if (recordedAtByChannel.isNotEmpty || appliedDeletion) {
        _publishUnreadCatchUp(recordedAtByChannel);
      }
    } catch (error) {
      if (!_lifecycleRef.mounted) return;
      debugPrint('[ChannelsNotifier] unread catch-up failed: $error');
    } finally {
      // Optional history can terminate without data. Release this destination
      // with its retained unread state, but never settle a retired request.
      if (_lifecycleRef.mounted &&
          !_isCatchUpRetired(fence, subscriptionGeneration)) {
        _settledUnreadGeneration = subscriptionGeneration;
        _notifyUnreadReadinessChanged();
      }
    }
  }

  /// Where a channel's catch-up starts.
  ///
  /// The first catch-up in a session starts after the read timestamp. Every
  /// refresh path catches up again (resume, reconnect, join), and on a slow
  /// link re-downloading up to [_unreadCatchUpLimit] messages per channel each
  /// time keeps the batch from ever finishing. A later catch-up therefore
  /// starts [relayLiveWindowSkewSeconds] before an earlier one was sent. The
  /// earlier one returned every message stored before it was sent, and the
  /// relay stores no event with a `created_at` more than 900s behind its own
  /// clock, so anything stored since then is newer than that start, however
  /// late it arrived. A read timestamp that moved back since then makes the
  /// full window apply again.
  int _unreadCatchUpSince(String channelId, int? readAt) {
    final afterRead = (readAt ?? -1) + 1;
    final caughtUp = _unreadCaughtUpByChannel[channelId];
    if (caughtUp == null || (readAt ?? -1) < (caughtUp.readAt ?? -1)) {
      return afterRead;
    }
    return max(afterRead, caughtUp.sentAt - relayLiveWindowSkewSeconds);
  }

  /// Remembers when a catch-up over [readAtByChannel] was sent, and the read
  /// timestamps it used, for every channel except [failedChannelIds]: their
  /// filters failed, so the next catch-up still needs their earlier window.
  void _recordUnreadCaughtUp(
    Map<String, int?> readAtByChannel,
    int sentAt,
    Set<String> failedChannelIds,
  ) {
    for (final MapEntry(key: channelId, value: readAt)
        in readAtByChannel.entries) {
      if (failedChannelIds.contains(channelId)) continue;
      _unreadCaughtUpByChannel[channelId] = (sentAt: sentAt, readAt: readAt);
    }
  }
}
