part of 'channels_provider.dart';

// Channels per gap-head deletion filter: half the relay's per-request `#h`
// cap, so two such filters fit in one request.
const _headDeletionChannels = kMaxExplicitChannelValues ~/ 2;

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
          for (final (kinds, since) in [
            (EventKind.channelMessageEventKinds, sinceByChannel[channel.id]!),
            (
              [EventKind.deletion, EventKind.nip29DeleteEvent],
              _deletionsSince(channel.id, sinceByChannel[channel.id]!),
            ),
          ])
            NostrFilter(
              kinds: kinds,
              tags: {
                '#h': [channel.id],
              },
              since: since,
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

      // A channel with a gap to read is unknown until that read answers, so a
      // newer refresh that retires this catch-up leaves the row visible.
      final unknownGaps = {...deferredGaps, ...gapFilters.keys};
      if (_activity.noteCatchUp(unknownGaps, unknownGaps)) {
        _publishUnreadCatchUp(const {});
      }
      final failedChannelIds = <String>{};
      final events = await _fetchChannelHistoryBatch(
        session,
        filters,
        operation: 'unread catch-up',
        failedChannelIds: failedChannelIds,
      );
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
      final activityCandidates = <NostrEvent>[];
      // Deletions from before a channel's own window come only from
      // [_deletionsSince], which asks for them to see whether the Active head
      // went. They are not new to the unread state, so they only reach it.
      final headDeletedIds = <String>{};
      for (final event in events) {
        final channelId = event.channelId;
        if (channelId != null &&
            event.createdAt < (sinceByChannel[channelId] ?? 0) &&
            (event.kind == EventKind.deletion ||
                event.kind == EventKind.nip29DeleteEvent)) {
          headDeletedIds.addAll(_deletionTargets(event));
          continue;
        }
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
        activityCandidates.add(event);
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
      // So does a channel whose activity became unknown. Newer activity and
      // channels that became known republish once their signatures are
      // checked.
      gapFilters.removeWhere((id, _) => failedChannelIds.contains(id));
      deferredGaps.removeAll(failedChannelIds);
      final coverageChanged = _activity.forgetHeads({
        ...batchDeletedIds,
        ...headDeletedIds,
      });
      if (recordedAtByChannel.isNotEmpty ||
          appliedDeletion ||
          coverageChanged) {
        _publishUnreadCatchUp(recordedAtByChannel);
      }
      // Activity waits on signature checks, so it lands after the unread
      // state rather than holding it back.
      unawaited(
        _noteCatchUpActivity(
          session,
          activityCandidates,
          catchUpChannelIds
              .difference(gapFilters.keys.toSet())
              .difference(deferredGaps),
          gapFilters,
          failedChannelIds,
          requestedAt,
          batchDeletedIds,
          fence,
          subscriptionGeneration,
        ),
      );
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

  /// Records the activity an unread catch-up found once its signatures are
  /// checked, and marks [covered] channels known, except [failedIds] and those
  /// whose verified head was deleted meanwhile. Then reads the [gaps] that a
  /// newer verified post has not made moot.
  Future<void> _noteCatchUpActivity(
    RelaySessionNotifier session,
    List<NostrEvent> candidates,
    Set<String> covered,
    Map<String, NostrFilter> gaps,
    Set<String> failedIds,
    int sentAt,
    Set<String> deletedIds,
    _ChannelRefreshFence fence,
    int subscriptionGeneration,
  ) async {
    ({Set<String> authentic, Set<String> rejected})? checked;
    try {
      checked = await _authenticActivityHeads(candidates, deletedIds);
    } catch (error) {
      debugPrint('[ChannelsNotifier] activity verification failed: $error');
    }
    if (!_lifecycleRef.mounted ||
        _isCatchUpRetired(fence, subscriptionGeneration)) {
      return;
    }
    void readGaps(Map<String, NostrFilter> gaps) {
      if (gaps.isEmpty) return;
      unawaited(
        _catchUpActivityGaps(
          session,
          gaps,
          sentAt,
          deletedIds,
          fence,
          subscriptionGeneration,
        ),
      );
    }

    if (checked == null) {
      if (_activity.noteCatchUp(covered, covered)) _publishUnreadCatchUp({});
      readGaps(gaps);
      return;
    }
    final authenticIds = checked.authentic;
    var changed = false;
    for (final event in candidates) {
      final channelId = event.channelId!;
      if (_unscopedUnreadDeletedIds.contains(event.id) ||
          _unreadDeletedIdsByChannel[channelId]?.contains(event.id) == true) {
        continue;
      }
      changed |= _activity.note(
        channelId,
        event,
        (event) => authenticIds.contains(event.id),
      );
    }
    final stale = {
      ...checked.rejected,
      ..._deletedHeadChannels(candidates, authenticIds, deletedIds),
    };
    // A verified post at or after a gap's end makes the gap moot.
    final moot = {
      for (final MapEntry(key: channelId, value: gap) in gaps.entries)
        if (!stale.contains(channelId) &&
            (_activity.latestAt(channelId) ?? -1) >= gap.until!)
          channelId,
    };
    changed |=
        _activity.noteCatchUp(
          covered.union(moot).difference(stale),
          failedIds,
          sentAt: sentAt,
        ) |
        _activity.noteCatchUp(stale, stale);
    if (changed) _publishUnreadCatchUp(const {});
    // A stale channel stays unknown until the next list read re-reads it.
    readGaps({
      for (final MapEntry(key: channelId, value: gap) in gaps.entries)
        if (!moot.contains(channelId) && !stale.contains(channelId))
          channelId: gap,
    });
  }

  /// Reads the newest post in each channel's [_activityGap], after the unread
  /// catch-up and apart from it, so its failure only leaves activity unknown.
  /// A failed gap is not retried: the next list read's catch-up reads it again.
  ///
  /// The unread catch-up asks for deletions only after the read marker, so
  /// one made in the gap while disconnected, and read past on another
  /// device, can still hold the activity head. This also asks for those of
  /// the heads: with the posts when they fit in one request, otherwise once
  /// the posts have settled, and only for the heads no gap post replaced, so
  /// a failed deletion read never costs the posts.
  Future<void> _catchUpActivityGaps(
    RelaySessionNotifier session,
    Map<String, NostrFilter> filtersByChannel,
    int sentAt,
    Set<String> deletedIds,
    _ChannelRefreshFence fence,
    int subscriptionGeneration,
  ) async {
    final channelIds = filtersByChannel.keys.toSet();
    final heads = {
      for (final channelId in channelIds)
        channelId: ?_activity.latestId(channelId),
    };
    final since = filtersByChannel.values
        .map((filter) => filter.since!)
        .reduce(min);
    final shared =
        channelIds.length + heads.length <= kMaxExplicitChannelValues;
    final failedChannelIds = <String>{};
    List<NostrEvent>? events;
    var headDeletions = const <NostrEvent>[];
    var authenticIds = const <String>{};
    var rejected = const <String>{};
    try {
      final read = await _fetchChannelHistoryBatch(
        session,
        [
          ...filtersByChannel.values,
          if (shared) ..._headDeletionFilters(heads, since),
        ],
        operation: 'activity gap read',
        failedChannelIds: failedChannelIds,
        optional: true,
      );
      if (read != null) {
        // A deletion can carry an `h` tag too, so it must not count as a post.
        final posts = <NostrEvent>[];
        final deletions = <NostrEvent>[];
        for (final event in read) {
          (event.kind == EventKind.deletion ||
                      event.kind == EventKind.nip29DeleteEvent
                  ? deletions
                  : posts)
              .add(event);
        }
        events = posts;
        headDeletions = deletions;
        (authentic: authenticIds, :rejected) = await _authenticActivityHeads(
          events,
          deletedIds,
        );
      }
    } catch (error) {
      // Unverified, the posts read cannot settle the gaps.
      events = null;
      debugPrint('[ChannelsNotifier] activity gap read failed: $error');
    }
    if (!_lifecycleRef.mounted ||
        _isCatchUpRetired(fence, subscriptionGeneration)) {
      return;
    }
    final deleted = {
      ...deletedIds,
      ..._deletedAmong(events),
      for (final deletion in headDeletions) ..._deletionTargets(deletion),
    };
    var changed = false;
    for (final event in events ?? const <NostrEvent>[]) {
      final channelId = event.channelId;
      if (channelId == null ||
          !channelIds.contains(channelId) ||
          deleted.contains(event.id)) {
        continue;
      }
      changed |= _activity.note(
        channelId,
        event,
        (event) => authenticIds.contains(event.id),
      );
    }
    // Each gap read asks for one post, so a deleted one hides whether an
    // older post in the gap survives.
    final failed = events == null
        ? channelIds
        : {
            ...failedChannelIds,
            ...rejected,
            for (final event in events)
              if (event.channelId case final channelId?
                  when deleted.contains(event.id))
                channelId,
          };
    // After the gap's posts, so a head they replaced stays known.
    changed |= _activity.forgetHeads(deleted);
    // The heads whose deletions are still unread stay unknown until they are.
    final unprobed = shared || events == null
        ? const <String>{}
        : {
            for (final MapEntry(key: channelId, value: id) in heads.entries)
              if (!failed.contains(channelId) &&
                  _activity.latestId(channelId) == id)
                channelId,
          };
    changed |= _activity.noteCatchUp(
      channelIds.difference(unprobed),
      failed,
      sentAt: sentAt,
    );
    if (changed) _publishUnreadCatchUp(const {});
    if (unprobed.isEmpty) return;

    List<NostrEvent>? probed;
    try {
      probed = await _fetchChannelHistoryBatch(
        session,
        _headDeletionFilters({
          for (final channelId in unprobed) channelId: heads[channelId]!,
        }, since),
        operation: 'activity head deletion read',
        optional: true,
      );
    } catch (error) {
      debugPrint(
        '[ChannelsNotifier] activity head deletion read failed: $error',
      );
    }
    if (!_lifecycleRef.mounted ||
        _isCatchUpRetired(fence, subscriptionGeneration)) {
      return;
    }
    final forgot = _activity.forgetHeads({
      for (final deletion in probed ?? const <NostrEvent>[])
        ..._deletionTargets(deletion),
    });
    if (forgot |
        _activity.noteCatchUp(
          unprobed,
          probed == null ? unprobed : const {},
          sentAt: sentAt,
        )) {
      _publishUnreadCatchUp(const {});
    }
  }

  /// Filters for deletions of each channel's [heads] made since [since],
  /// [_headDeletionChannels] channels per filter.
  List<NostrFilter> _headDeletionFilters(Map<String, String> heads, int since) {
    final entries = heads.entries.toList();
    return [
      for (
        var start = 0;
        start < entries.length;
        start += _headDeletionChannels
      )
        NostrFilter(
          kinds: const [EventKind.deletion, EventKind.nip29DeleteEvent],
          tags: {
            '#h': [
              for (final head
                  in entries.skip(start).take(_headDeletionChannels))
                head.key,
            ],
            '#e': [
              for (final head
                  in entries.skip(start).take(_headDeletionChannels))
                head.value,
            ],
          },
          since: since,
          limit: _unreadCatchUpLimit,
        ),
    ];
  }

  /// [_ChannelActivity.authenticHeadIds] of [events], skipping posts that are
  /// deleted, in [deletedIds], by a deletion among [events] or by a deletion
  /// seen before, since those never count.
  Future<({Set<String> authentic, Set<String> rejected})>
  _authenticActivityHeads(List<NostrEvent> events, Set<String> deletedIds) =>
      _ChannelActivity.authenticHeadIds(events, {
        ...deletedIds,
        for (final event in events)
          if (event.kind == EventKind.deletion ||
              event.kind == EventKind.nip29DeleteEvent)
            ..._deletionTargets(event),
        ..._unscopedUnreadDeletedIds,
        for (final ids in _unreadDeletedIdsByChannel.values) ...ids,
      });

  /// The channels whose verified head among [events] was deleted, in
  /// [deletedIds] or by a deletion seen since it was verified. Their older
  /// posts went unchecked, so the read leaves them unknown.
  Set<String> _deletedHeadChannels(
    List<NostrEvent> events,
    Set<String> authenticIds,
    Set<String> deletedIds,
  ) => {
    for (final event in events)
      if (event.channelId case final channelId?
          when authenticIds.contains(event.id) &&
              (deletedIds.contains(event.id) ||
                  _unscopedUnreadDeletedIds.contains(event.id) ||
                  _unreadDeletedIdsByChannel[channelId]?.contains(event.id) ==
                      true))
        channelId,
  };

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

  /// Where [channelId]'s catch-up asks for deletions. When an Active section
  /// shows it and its latest post is from the second before [since], as when
  /// the read marker sits on that post, the gap read is moot, so a deletion
  /// of that post made in the same second is asked for here.
  int _deletionsSince(String channelId, int since) =>
      _activity.isWatched(channelId) &&
          _activity.latestAt(channelId) == since - 1
      ? since - 1
      : since;

  /// The stretch a catch-up starting at [since] skips since the last read
  /// that covered the channel, for the Active filter: a post that arrived
  /// while disconnected and was read on another device lies there, as does one
  /// before an unread cursor reset. The catch-up asks for its newest post,
  /// and for deletions of the latest post known made since it began.
  /// A channel known only from a live post counts as covered at that post.
  /// Null when nothing covered the channel yet or nothing is skipped.
  ({int since, int until})? _activityGap(String channelId, int since) {
    final coveredAt =
        _activity.coveredAt(channelId) ?? _activity.latestAt(channelId);
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
