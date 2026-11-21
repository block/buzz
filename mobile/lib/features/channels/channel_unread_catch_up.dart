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
    // A catch-up that misses channels is retried, as a list read.
    var incomplete = false;
    var catchUpChannelIds = const <String>{};
    try {
      // Before any read that can throw, so a failed setup marks them unknown.
      final activeChannels = [
        for (final channel in channels)
          if (channel.isMember && !channel.isArchived) channel,
      ];
      final channelById = {
        for (final channel in activeChannels) channel.id: channel,
      };
      catchUpChannelIds = channelById.keys.toSet();
      final myPk = _lifecycleRef.read(myPubkeyProvider);
      if (myPk == null) return;

      final session = _lifecycleRef.read(relaySessionProvider.notifier);
      final mutedChannelIds = _mutedChannelIds();
      final readState = _lifecycleRef.read(readStateProvider);
      final readAtByChannel = {
        for (final channel in activeChannels)
          channel.id: readState.effectiveTimestamp(channel.id),
      };
      final requestedAt = currentUnixSeconds();
      final cursorsAtStart = {
        for (final channel in activeChannels)
          channel.id: _unreadCaughtUpByChannel[channel.id],
      };
      final sinceByChannel = {
        for (final channel in activeChannels)
          channel.id: _unreadCatchUpSince(
            channel.id,
            readAtByChannel[channel.id],
            muted: mutedChannelIds.contains(channel.id),
          ),
      };
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
              since: sinceByChannel[channel.id],
              limit: _unreadCatchUpLimit,
            ),
      ];
      // A gap only matters to the Active filter, and only when a post in it
      // could be newer than the activity already known. A channel no Active
      // section shows keeps its gap for a later catch-up, and reads as
      // unknown meanwhile.
      final gapFilters = <String, NostrFilter>{};
      final deferredGaps = <String>{};
      for (final channel in activeChannels) {
        final gap = _activityGap(channel.id, sinceByChannel[channel.id]!);
        if (gap == null) continue;
        if ((_activity.latestAt(channel.id) ?? -1) >= gap.until) continue;
        if (!_activity.isWatched(channel.id)) {
          deferredGaps.add(channel.id);
          continue;
        }
        gapFilters[channel.id] = NostrFilter(
          kinds: EventKind.channelMessageEventKinds,
          tags: {
            '#h': [channel.id],
          },
          since: gap.since,
          until: gap.until,
          limit: 1,
        );
      }

      final failedChannelIds = <String>{};
      final events = await _fetchChannelHistoryBatch(
        session,
        filters,
        operation: 'unread catch-up',
        failedChannelIds: failedChannelIds,
      );
      final authenticIds = events == null
          ? const <String>{}
          : await _authenticActivityHeads(events, const {});
      // The relay round-trip above is the window Jed's probes park in: a newer
      // refresh, a community switch or an identity switch here means every
      // write below belongs to a channel list the user has left.
      if (!_lifecycleRef.mounted ||
          _isCatchUpRetired(fence, subscriptionGeneration)) {
        return;
      }
      // Unavailable: keep the unread state already observed, but not the
      // activity, which may have missed a post.
      if (events == null) {
        incomplete = true;
        if (_activity.noteCatchUp(catchUpChannelIds, catchUpChannelIds)) {
          _publishUnreadCatchUp(const {});
        }
        return;
      }

      _recordUnreadCaughtUp(
        readAtByChannel,
        requestedAt,
        failedChannelIds,
        cursorsAtStart: cursorsAtStart,
        mutedChannelIds: mutedChannelIds,
      );
      incomplete = failedChannelIds.isNotEmpty;

      // Keep this response's deletion evidence independently of the bounded
      // cross-request tombstones: a large batch can evict its own early IDs.
      final batchDeletedIds = <String>{};
      var appliedDeletion = false;
      var activityAdvanced = false;
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

      var threadInterestChanged = false;
      for (final event in events) {
        if (batchDeletedIds.contains(event.id)) continue;
        if (EventKind.channelMessageEventKinds.contains(event.kind) &&
            event.pubkey.toLowerCase() == myPk.toLowerCase() &&
            _addSelfThreadInterest(event)) {
          threadInterestChanged = true;
        }
      }
      if (threadInterestChanged) _writeThreadInterestStores(myPk);

      final recordedAtByChannel = <String, int>{};
      final windowSizeByChannel = <String, int>{};
      final oldestQuietByChannel = <String, NostrEvent>{};
      for (final event in events) {
        if (!EventKind.channelMessageEventKinds.contains(event.kind)) continue;
        final channelId = event.channelId;
        if (channelId == null) continue;
        final channel = channelById[channelId];
        if (channel == null) continue;
        final readAt = readAtByChannel[channelId];
        // Read posts say nothing about whether the window hides an older
        // unread one.
        if (readAt == null || event.createdAt > readAt) {
          windowSizeByChannel.update(
            channelId,
            (n) => n + 1,
            ifAbsent: () => 1,
          );
        }
        if (batchDeletedIds.contains(event.id) ||
            _unscopedUnreadDeletedIds.contains(event.id) ||
            _unreadDeletedIdsByChannel[channelId]?.contains(event.id) == true) {
          continue;
        }
        if (_activity.note(
          channelId,
          event,
          (event) => authenticIds.contains(event.id),
        )) {
          activityAdvanced = true;
        }
        _noteThreadChannel(event, channelId);
        if (event.pubkey.toLowerCase() == myPk.toLowerCase()) continue;
        if (readAt != null && event.createdAt <= readAt) continue;
        if (!shouldNotifyForEvent(
          event,
          myPk,
          participatedRootIds: _participatedRootIds,
          followedRootIds: _followedRootIds(),
          authoredRootIds: _authoredRootIds,
          mutedRootIds: _unfollowedRootIds(channel),
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
      // Newer activity also republishes, so the Active filter sees it.
      // So does a channel whose activity became known or unknown.
      // A channel with a gap to read stays as it was until that read answers.
      gapFilters.removeWhere((id, _) => failedChannelIds.contains(id));
      deferredGaps.removeAll(failedChannelIds);
      final coverageChanged =
          _activity.noteCatchUp(
            catchUpChannelIds
                .difference(gapFilters.keys.toSet())
                .difference(deferredGaps),
            failedChannelIds,
            sentAt: requestedAt,
          ) |
          _activity.noteCatchUp(deferredGaps, deferredGaps) |
          _activity.forgetHeads(batchDeletedIds);
      if (recordedAtByChannel.isNotEmpty ||
          appliedDeletion ||
          activityAdvanced ||
          coverageChanged) {
        _publishUnreadCatchUp(recordedAtByChannel);
      }
      if (gapFilters.isNotEmpty) {
        unawaited(
          _catchUpActivityGaps(
            session,
            gapFilters,
            requestedAt,
            batchDeletedIds,
            fence,
            subscriptionGeneration,
          ),
        );
      }
    } catch (error) {
      incomplete = true;
      if (!_lifecycleRef.mounted) return;
      debugPrint('[ChannelsNotifier] unread catch-up failed: $error');
      if (!_isCatchUpRetired(fence, subscriptionGeneration) &&
          _activity.noteCatchUp(catchUpChannelIds, catchUpChannelIds)) {
        _publishUnreadCatchUp(const {});
      }
    } finally {
      // Optional history can terminate without data. Release this destination
      // with its retained unread state, but never settle a retired request.
      if (_lifecycleRef.mounted &&
          !_isCatchUpRetired(fence, subscriptionGeneration)) {
        _settledUnreadGeneration = subscriptionGeneration;
        _notifyUnreadReadinessChanged();
        if (incomplete) {
          _unreadCatchUpRetry.schedule(_retryUnreadCatchUp);
        } else {
          _unreadCatchUpRetry.cancel();
        }
      }
    }
  }

  /// Reads the newest post in each channel's [_activityGap], after the unread
  /// catch-up and apart from it, so its failure only leaves activity unknown.
  /// A failed gap is not retried: the next list read's catch-up reads it again.
  Future<void> _catchUpActivityGaps(
    RelaySessionNotifier session,
    Map<String, NostrFilter> filtersByChannel,
    int sentAt,
    Set<String> deletedIds,
    _ChannelRefreshFence fence,
    int subscriptionGeneration,
  ) async {
    final channelIds = filtersByChannel.keys.toSet();
    final failedChannelIds = <String>{};
    List<NostrEvent>? events;
    var authenticIds = const <String>{};
    try {
      events = await _fetchChannelHistoryBatch(
        session,
        filtersByChannel.values.toList(),
        operation: 'activity gap read',
        failedChannelIds: failedChannelIds,
        optional: true,
      );
      if (events != null) {
        authenticIds = await _authenticActivityHeads(events, deletedIds);
      }
    } catch (error) {
      debugPrint('[ChannelsNotifier] activity gap read failed: $error');
    }
    if (!_lifecycleRef.mounted ||
        _isCatchUpRetired(fence, subscriptionGeneration)) {
      return;
    }
    var changed = false;
    for (final event in events ?? const <NostrEvent>[]) {
      final channelId = event.channelId;
      if (channelId == null ||
          !channelIds.contains(channelId) ||
          deletedIds.contains(event.id) ||
          _unscopedUnreadDeletedIds.contains(event.id) ||
          _unreadDeletedIdsByChannel[channelId]?.contains(event.id) == true) {
        continue;
      }
      changed |= _activity.note(
        channelId,
        event,
        (event) => authenticIds.contains(event.id),
      );
    }
    final failed = events == null ? channelIds : failedChannelIds;
    changed |= _activity.noteCatchUp(channelIds, failed, sentAt: sentAt);
    if (changed) _publishUnreadCatchUp(const {});
  }

  /// [_ChannelActivity.authenticHeadIds] of [events], skipping posts that are
  /// deleted, in [deletedIds], by a deletion among [events] or by a deletion
  /// seen before, since those never count.
  Future<Set<String>> _authenticActivityHeads(
    List<NostrEvent> events,
    Set<String> deletedIds,
  ) => _ChannelActivity.authenticHeadIds(events, {
    ...deletedIds,
    for (final event in events)
      if (event.kind == EventKind.deletion ||
          event.kind == EventKind.nip29DeleteEvent)
        ..._deletionTargets(event),
    ..._unscopedUnreadDeletedIds,
    for (final ids in _unreadDeletedIdsByChannel.values) ...ids,
  });

  /// Where a channel's catch-up starts.
  ///
  /// The first catch-up in a session starts after the read timestamp. Every
  /// refresh path catches up again (resume, reconnect, join), and on a slow
  /// link re-downloading up to [_unreadCatchUpLimit] messages per channel each
  /// time keeps the batch from ever finishing. A later catch-up therefore
  /// starts [relayCrossDeviceSkewSeconds] before an earlier one was sent. The
  /// earlier one returned every message stored before it was sent, and the
  /// relay stores no event with a `created_at` more than 900s from its own
  /// clock, so anything stored since then is newer than that start, however
  /// late it arrived and however far this phone's clock is from the sender's. A read timestamp that moved back since then, or a mute
  /// change (which changes what counts as unread), makes the full window apply
  /// again.
  int _unreadCatchUpSince(
    String channelId,
    int? readAt, {
    required bool muted,
  }) {
    final afterRead = (readAt ?? -1) + 1;
    final caughtUp = _unreadCaughtUpByChannel[channelId];
    if (caughtUp == null ||
        caughtUp.muted != muted ||
        (readAt ?? -1) < (caughtUp.readAt ?? -1)) {
      return afterRead;
    }
    return max(afterRead, caughtUp.sentAt - relayCrossDeviceSkewSeconds);
  }

  /// The stretch a catch-up starting at [since] skips since the last read
  /// that covered the channel, for the Active filter: a post that arrived
  /// while disconnected and was read on another device lies there, as does one
  /// before an unread cursor reset. The catch-up asks for its newest post.
  /// Null when no read covered the channel yet or nothing is skipped.
  ({int since, int until})? _activityGap(String channelId, int since) {
    final coveredAt = _activity.coveredAt(channelId);
    if (coveredAt == null) return null;
    final from = coveredAt - relayCrossDeviceSkewSeconds;
    return since > from ? (since: from, until: since - 1) : null;
  }

  /// Remembers when a catch-up over [readAtByChannel] was sent, and the read
  /// timestamps and mute state it used, for every channel except
  /// [failedChannelIds]: their filters failed, so the next catch-up still needs
  /// their earlier window. A channel whose cursor changed while the request was
  /// out, such as a live deletion of its marker, keeps that change: this
  /// response only covered the window the old cursor asked for.
  void _recordUnreadCaughtUp(
    Map<String, int?> readAtByChannel,
    int sentAt,
    Set<String> failedChannelIds, {
    required Map<String, ({int sentAt, int? readAt, bool muted})?>
    cursorsAtStart,
    required Set<String> mutedChannelIds,
  }) {
    for (final MapEntry(key: channelId, value: readAt)
        in readAtByChannel.entries) {
      if (failedChannelIds.contains(channelId)) continue;
      final atStart = cursorsAtStart[channelId];
      if (atStart != null && _unreadCaughtUpByChannel[channelId] != atStart) {
        continue;
      }
      _unreadCaughtUpByChannel[channelId] = (
        sentAt: sentAt,
        readAt: readAt,
        muted: mutedChannelIds.contains(channelId),
      );
    }
  }
}

Channel _advanceLastMessageAt(Channel channel, int? createdAt) {
  if (createdAt == null) return channel;
  final at = DateTime.fromMillisecondsSinceEpoch(createdAt * 1000, isUtc: true);
  final current = channel.lastMessageAt;
  if (current != null && !at.isAfter(current)) return channel;
  return channel.copyWith(lastMessageAt: at);
}
