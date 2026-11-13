part of 'channels_provider.dart';

/// A channel's unread catch-up cursor: when it was sent, and what counted as
/// unread then (the read timestamp, the mute and the followed threads).
/// [verify] asks the next catch-up to check the channel's unread events by ID,
/// because a full page of deletions may have left some of theirs out.
typedef _UnreadCursor = ({
  int sentAt,
  int? readAt,
  bool muted,
  int follows,
  bool verify,
});

/// A fingerprint of followed thread roots that stays the same across launches,
/// unlike [Object.hash] of strings.
int _followsFingerprint(Set<String> rootIds) {
  var hash = 0x811c9dc5;
  for (final id in rootIds.toList()..sort()) {
    for (final unit in [...id.codeUnits, 0]) {
      hash = ((hash ^ unit) * 0x01000193) & 0xffffffff;
    }
  }
  return hash;
}

/// A followed-threads value no fingerprint has, so the cursor it marks reads
/// the full window, and replaces its evidence, on the next catch-up.
const _staleFollows = -1;

extension on _UnreadCursor {
  _UnreadCursor withFollows(int follows) => (
    sentAt: sentAt,
    readAt: readAt,
    muted: muted,
    follows: follows,
    verify: verify,
  );

  _UnreadCursor withVerify() => (
    sentAt: sentAt,
    readAt: readAt,
    muted: muted,
    follows: follows,
    verify: true,
  );
}

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
      final follows = _followsFingerprint(_followedRootIds());
      final sinceByChannel = {
        for (final channel in activeChannels)
          channel.id: _unreadCatchUpSince(
            channel.id,
            readAtByChannel[channel.id],
            muted: mutedChannelIds.contains(channel.id),
            follows: follows,
          ),
      };
      // Channels whose cursor no longer holds read the whole unread window,
      // and it replaces what was observed before it was sent. Live events can
      // arrive while it is out; they are not in this set. A channel without a
      // cursor yet only adds what its window finds.
      final observedBeforeByChannel = {
        for (final MapEntry(key: channelId, value: since)
            in sinceByChannel.entries)
          if (_unreadCaughtUpByChannel[channelId] != null &&
              since == (readAtByChannel[channelId] ?? -1) + 1)
            channelId: {...?_observedUnreadEventsByChannel[channelId]?.keys},
      };
      final cursorsAtStart = {
        for (final channel in activeChannels)
          channel.id: _unreadCaughtUpByChannel[channel.id],
      };
      final pendingRepliesByChannel = {
        for (final channel in activeChannels)
          if (_pendingThreadReplyIds(channel.id) case final ids
              when ids.isNotEmpty)
            channel.id: ids,
      };
      // Unread events a full deletion page left unconfirmed: what the relay
      // no longer returns by ID was deleted.
      final verifyByChannel = {
        for (final channel in activeChannels)
          if (cursorsAtStart[channel.id]?.verify == true)
            channel.id: {
              for (final event
                  in _observedUnreadEventsByChannel[channel.id]?.values ??
                      const <ObservedUnreadEvent>[])
                if (!event.threadReplyPending) event.id,
            },
      };
      final byIdByChannel = {
        for (final channelId in {
          ...pendingRepliesByChannel.keys,
          ...verifyByChannel.keys,
        })
          if ({
                ...?pendingRepliesByChannel[channelId],
                ...?verifyByChannel[channelId],
              }
              case final ids when ids.isNotEmpty)
            channelId: ids,
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
        // Replies restored from disk without their payload, which Activity
        // shows, and events to confirm. They are older than the window above.
        for (final MapEntry(key: channelId, value: ids)
            in byIdByChannel.entries)
          NostrFilter(
            ids: ids.toList(),
            kinds: EventKind.channelMessageEventKinds,
            tags: {
              '#h': [channelId],
            },
            limit: ids.length,
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
      if (events == null) {
        incomplete = true;
        return;
      }
      // The relay round-trip above is the window Jed's probes park in: a newer
      // refresh, a community switch or an identity switch here means every
      // write below belongs to a channel list the user has left.
      if (!_lifecycleRef.mounted ||
          _isCatchUpRetired(fence, subscriptionGeneration)) {
        return;
      }

      _recordUnreadCaughtUp(
        readAtByChannel,
        requestedAt,
        failedChannelIds,
        cursorsAtStart: cursorsAtStart,
        mutedChannelIds: mutedChannelIds,
        follows: follows,
      );
      incomplete = failedChannelIds.isNotEmpty;

      // Keep this response's deletion evidence independently of the bounded
      // cross-request tombstones: a large batch can evict its own early IDs.
      final batchDeletedIds = <String>{};
      var appliedDeletion = false;
      final cursorsBeforeDeletions = Map.of(_unreadCaughtUpByChannel);
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

      // A tombstone that dropped saved evidence stales its channel's cursor
      // (see _applyUnreadDeletion): read that full window soon, as a retry.
      if (_unreadCaughtUpByChannel.entries.any(
        (entry) => cursorsBeforeDeletions[entry.key] != entry.value,
      )) {
        incomplete = true;
      }

      // A full page of deletions may have left older ones out, and a restored
      // event has nothing else to say it was deleted. NIP-09 deletions can
      // omit h, so those count toward every channel. The next catch-up, soon
      // as a retry, checks the channel's unread events by ID instead, and
      // skips this check for it, so it does not repeat itself. A window read
      // from scratch has nothing restored to check.
      final deletionsByChannel = <String?, int>{};
      for (final event in events) {
        if (event.kind != EventKind.deletion &&
            event.kind != EventKind.nip29DeleteEvent) {
          continue;
        }
        deletionsByChannel.update(
          event.channelId,
          (n) => n + 1,
          ifAbsent: () => 1,
        );
      }
      for (final channelId in channelById.keys) {
        if (cursorsAtStart[channelId] == null ||
            verifyByChannel.containsKey(channelId) ||
            failedChannelIds.contains(channelId)) {
          continue;
        }
        if ((deletionsByChannel[channelId] ?? 0) +
                (deletionsByChannel[null] ?? 0) <
            _unreadCatchUpLimit) {
          continue;
        }
        if (_unreadCaughtUpByChannel[channelId] case final cursor?) {
          _unreadCaughtUpByChannel[channelId] = cursor.withVerify();
          incomplete = true;
        }
      }

      for (final event in events) {
        if (batchDeletedIds.contains(event.id)) continue;
        if (EventKind.channelMessageEventKinds.contains(event.kind) &&
            event.pubkey.toLowerCase() == myPk.toLowerCase()) {
          _recordSelfThreadInterest(event, myPk);
        }
      }

      // The window each channel's filter returned: every unique message at or
      // after its start, including a pending reply the window also matched.
      // A reply fetched by ID from before the start is not part of it.
      final windowIdsByChannel = <String, Set<String>>{};
      final windowStartByChannel = <String, int>{};
      for (final event in events) {
        if (!EventKind.channelMessageEventKinds.contains(event.kind)) continue;
        final channelId = event.channelId;
        final since = sinceByChannel[channelId];
        if (channelId == null || since == null || event.createdAt < since) {
          continue;
        }
        windowIdsByChannel.putIfAbsent(channelId, () => {}).add(event.id);
        windowStartByChannel[channelId] = min(
          windowStartByChannel[channelId] ?? event.createdAt,
          event.createdAt,
        );
      }

      // A full window replaces the channel's earlier evidence over the span it
      // covered, so what no longer counts (a mute, an unfollow, a deletion
      // the delta missed) goes, and what still counts is recorded again
      // below, before anything is published. A window that hit its limit
      // covers only back to its oldest message; older evidence stays only if
      // its saved flags still earn attention. Pending replies are checked by
      // their own fetch.
      final followedRootIds = _followedRootIds();
      // Every DM event is saved as high priority, so in a muted DM the flag
      // can't tell a mention from an ordinary message: none is kept.
      bool stillCounts(
        ObservedUnreadEvent event, {
        required bool muted,
        required bool dm,
      }) {
        if (muted) return event.highPriority && !dm;
        if (event.highPriority) return true;
        final rootId = event.rootId;
        return rootId == null ||
            followedRootIds.contains(rootId) ||
            _participatedRootIds.contains(rootId) ||
            _authoredRootIds.contains(rootId);
      }

      var replaced = false;
      for (final MapEntry(key: channelId, value: observedBefore)
          in observedBeforeByChannel.entries) {
        if (failedChannelIds.contains(channelId)) continue;
        final saturated =
            (windowIdsByChannel[channelId]?.length ?? 0) >= _unreadCatchUpLimit;
        final from = saturated ? windowStartByChannel[channelId]! : null;
        final muted = mutedChannelIds.contains(channelId);
        replaced |= _removeObservedUnreadEvents(
          channelId,
          (event) =>
              !event.threadReplyPending &&
              observedBefore.contains(event.id) &&
              (from == null ||
                  event.createdAt >= from ||
                  !stillCounts(
                    event,
                    muted: muted,
                    dm: channelById[channelId]?.isDm ?? false,
                  )),
        );
      }

      final recordedAtByChannel = <String, int>{};
      final oldestQuietByChannel = <String, NostrEvent>{};
      for (final event in events) {
        if (!EventKind.channelMessageEventKinds.contains(event.kind)) continue;
        final channelId = event.channelId;
        if (channelId == null) continue;
        final channel = channelById[channelId];
        if (channel == null) continue;
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
        if ((windowIdsByChannel[channelId]?.length ?? 0) <
            _unreadCatchUpLimit) {
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
      // A requested reply that is gone, or no longer earns attention, would
      // not have been found by a full window either.
      var droppedPending = false;
      for (final MapEntry(key: channelId, value: ids)
          in pendingRepliesByChannel.entries) {
        if (failedChannelIds.contains(channelId)) continue;
        droppedPending |= _dropPendingThreadReplies(channelId, ids);
      }
      final returnedIds = {for (final event in events) event.id};
      for (final MapEntry(key: channelId, value: ids)
          in verifyByChannel.entries) {
        if (failedChannelIds.contains(channelId)) continue;
        final dropped = _removeObservedUnreadEvents(
          channelId,
          (event) =>
              !event.threadReplyPending &&
              ids.contains(event.id) &&
              !returnedIds.contains(event.id),
        );
        // As for a returned tombstone: a deleted marker may have been standing
        // in for a full window of quiet messages, so read the window again,
        // soon, as a retry.
        if (dropped) {
          if (_unreadCaughtUpByChannel[channelId] case final cursor?) {
            _unreadCaughtUpByChannel[channelId] = cursor.withFollows(
              _staleFollows,
            );
          }
          incomplete = true;
        }
        droppedPending |= dropped;
      }
      // Republish only when this catch-up actually changed unread state. A
      // batch that recorded nothing has nothing to show, and a failed or
      // superseded batch must not repaint another refresh's list: the retired
      // check above already returned in that case, and no await separates it
      // from here, so a second check would be dead code.
      // Recorded events also advance `lastMessageAt`, as live events do: a
      // deadline-unavailable latest-message batch leaves it unset.
      if (recordedAtByChannel.isNotEmpty ||
          appliedDeletion ||
          droppedPending ||
          replaced) {
        _publishUnreadCatchUp(recordedAtByChannel);
      }
      // Saved after every settled catch-up, even one that found nothing: its
      // cursors moved, and the next launch can start from them.
      _saveUnreadSnapshot?.call();
    } catch (error) {
      incomplete = true;
      if (!_lifecycleRef.mounted) return;
      debugPrint('[ChannelsNotifier] unread catch-up failed: $error');
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
  /// late it arrived and however far this phone's clock is from the sender's.
  /// A read timestamp that moved back since then, or a change to the mute or
  /// the followed threads (which change what counts as unread), makes the full
  /// window apply again.
  int _unreadCatchUpSince(
    String channelId,
    int? readAt, {
    required bool muted,
    required int follows,
  }) {
    final afterRead = (readAt ?? -1) + 1;
    final caughtUp = _unreadCaughtUpByChannel[channelId];
    if (caughtUp == null ||
        caughtUp.muted != muted ||
        caughtUp.follows != follows ||
        (readAt ?? -1) < (caughtUp.readAt ?? -1)) {
      return afterRead;
    }
    return max(afterRead, caughtUp.sentAt - relayCrossDeviceSkewSeconds);
  }

  /// Remembers when a catch-up over [readAtByChannel] was sent, and the read
  /// timestamps, mute state and followed threads it used, for every channel
  /// except [failedChannelIds]: their filters failed, so the next catch-up
  /// still needs their earlier window. A channel whose cursor changed while
  /// the request was out, such as a live deletion of its marker, keeps that
  /// change: this response only covered the window the old cursor asked for.
  void _recordUnreadCaughtUp(
    Map<String, int?> readAtByChannel,
    int sentAt,
    Set<String> failedChannelIds, {
    required Map<String, _UnreadCursor?> cursorsAtStart,
    required Set<String> mutedChannelIds,
    required int follows,
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
        follows: follows,
        verify: false,
      );
    }
  }
}
