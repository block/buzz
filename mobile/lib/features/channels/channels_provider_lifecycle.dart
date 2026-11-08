part of 'channels_provider.dart';

// Mirrors MAX_EXPLICIT_CHANNEL_VALUES in the relay REQ handler. A larger live
// set must use multiple subscriptions or the relay rejects the whole REQ.
const _maxLiveChannelsPerSubscription = 128;

/// Owns live channel subscription reconciliation and teardown.
///
/// Keeping this extension in a part preserves access to the notifier's private
/// lifecycle state while keeping `channels_provider.dart` below the mobile
/// file-size ratchet.
extension _ChannelsNotifierLiveSubscriptions on ChannelsNotifier {
  /// Subscribe to live events in deterministic chunks that stay within the
  /// relay's aggregate explicit-`#h` limit, plus the channel list notices in
  /// [_ChannelListChanges], whose events schedule a read of the list.
  Future<void> _subscribeLive(
    List<Channel> channels,
    _ChannelRefreshFence fence,
  ) {
    final channelIds = {
      for (final channel in channels)
        if (channel.isMember && !channel.isArchived) channel.id,
    };
    final relayBaseUrl = _lifecycleRef.read(relayConfigProvider).baseUrl;
    _desiredLiveChannelIds = channelIds;
    // A changed filter may be admitted, but an unchanged terminal rejection
    // cannot recover just because a timer or manual refresh fetched it again.
    // Drop obsolete keys so this quarantine stays bounded by desired chunks.
    final desiredKeys = chunkChannelIdsForLiveSubscriptions(
      channelIds,
    ).map(_liveChunkKey).toSet();
    _terminallyClosedLiveChunks.retainAll(desiredKeys);
    _refusedLiveChunks.retainAll(desiredKeys);
    _liveFence = fence;
    final subscriptionVersion = ++_subscriptionVersion;

    final sync = _liveSubscriptionQueue.then(
      (_) => _syncLiveSubscriptions(
        relayBaseUrl,
        subscriptionVersion,
        channels,
        fence,
      ),
    );
    _liveSubscriptionQueue = sync
        .whenComplete(() {
          // Reconcile even if a retired generation exits before its successor
          // can run (for example, a disconnect while subscribe is pending).
          if (_lifecycleRef.mounted) _removeUndesiredLiveChunks();
        })
        .catchError((Object error, StackTrace stack) {
          if (error is! _StaleChannelRefresh) {
            debugPrint(
              '[ChannelsNotifier] live subscription sync failed: $error\n$stack',
            );
          }
        });
    _notifyUnreadReadinessChanged();
    return _liveSubscriptionQueue;
  }

  Future<void> _syncLiveSubscriptions(
    String relayBaseUrl,
    int subscriptionVersion,
    List<Channel> channels,
    _ChannelRefreshFence fence,
  ) async {
    if (!_lifecycleRef.mounted) return;
    fence.ensureCurrent();
    if (_lifecycleRef.read(relaySessionProvider).status !=
            SessionStatus.connected ||
        subscriptionVersion != _subscriptionVersion) {
      return;
    }

    if (_subscriptionRelayBaseUrl != relayBaseUrl) {
      _clearRetainedLiveChunks();
      _subscriptionRelayBaseUrl = relayBaseUrl;
    }
    if (_lifecycleRef.read(relayConfigProvider).baseUrl != relayBaseUrl) return;

    final desiredChunks = chunkChannelIdsForLiveSubscriptions(
      _desiredLiveChannelIds,
    );
    // Keep existing coverage while replacements are installed. Cleanup runs
    // when this generation settles, and retains old coverage on failures.

    for (final chunk in desiredChunks) {
      final chunkKey = _liveChunkKey(chunk);
      if (_liveSubscriptionsByChunk.containsKey(chunkKey) ||
          _terminallyClosedLiveChunks.contains(chunkKey) ||
          _refusedLiveChunks.contains(chunkKey)) {
        continue;
      }
      if (!await _subscribeLiveChunk(
        chunk,
        relayBaseUrl,
        subscriptionVersion,
        fence,
      )) {
        return;
      }
    }

    if (!_lifecycleRef.mounted ||
        _lifecycleRef.read(relaySessionProvider).status !=
            SessionStatus.connected ||
        subscriptionVersion != _subscriptionVersion) {
      return;
    }

    fence.ensureCurrent();
    _unreadCatchUp = _catchUpUnreadEvents(channels, fence, subscriptionVersion);
    _notifyUnreadReadinessChanged();
    unawaited(_unreadCatchUp);
  }

  /// Subscribes one live chunk. Returns false when the sync should stop.
  Future<bool> _subscribeLiveChunk(
    List<String> chunk,
    String relayBaseUrl,
    int subscriptionVersion,
    _ChannelRefreshFence fence,
  ) async {
    final chunkKey = _liveChunkKey(chunk);
    final session = _lifecycleRef.read(relaySessionProvider.notifier);
    if (_lifecycleRef.read(relaySessionProvider).status !=
        SessionStatus.connected) {
      return false;
    }
    final generation = ++_nextLiveChunkGeneration;
    final subscription = _LiveChunkSubscription(generation);
    _liveSubscriptionsByChunk[chunkKey] = subscription;
    try {
      final unsubscribe = await session.subscribeWithStatus(
        NostrFilter(
          // Metadata edits say a listed channel was renamed or redescribed.
          kinds: [...EventKind.channelEventKinds, EventKind.nip29EditMetadata],
          tags: {'#h': chunk},
          limit: 0,
        ),
        (event) {
          // Superseded chunks can overlap during replacement or recovery.
          // Deliver only the current scope's still-desired channels; duplicate
          // events are already idempotent in the unread/timestamp stores.
          if (!_lifecycleRef.mounted ||
              _refreshCoordinator.currentScope() != fence.scope ||
              !(_desiredLiveChannelIds.contains(event.channelId) ||
                  (event.kind == EventKind.deletion &&
                      event.channelId == null &&
                      chunk.any(_desiredLiveChannelIds.contains)))) {
            return;
          }
          _handleLiveEvent(event);
        },
        onClosed: (message) =>
            _handleLiveChunkClosed(chunkKey, generation, message),
        // RelaySession reopens a REQ the relay closed for a transient reason
        // itself, and reports it only here.
        onStatusChanged: (status) {
          if (!_lifecycleRef.mounted ||
              _liveSubscriptionsByChunk[chunkKey] != subscription) {
            return;
          }
          if (status == RelaySubscriptionStatus.retrying) {
            subscription.interrupted = true;
          } else if (subscription.interrupted) {
            subscription.interrupted = false;
            _reopenedLiveChunks.add(chunkKey);
            _scheduleListRead();
          }
        },
      );
      subscription.unsubscribe = unsubscribe;
      if (!_lifecycleRef.mounted || !fence.isCurrent) {
        _liveSubscriptionsByChunk.remove(chunkKey);
        unsubscribe();
        if (!fence.isCurrent) throw const _StaleChannelRefresh();
        return false;
      }
      if (subscriptionVersion != _subscriptionVersion ||
          _lifecycleRef.read(relaySessionProvider).status !=
              SessionStatus.connected ||
          _lifecycleRef.read(relayConfigProvider).baseUrl != relayBaseUrl ||
          _subscriptionRelayBaseUrl != relayBaseUrl ||
          !chunk.every(_desiredLiveChannelIds.contains)) {
        _liveSubscriptionsByChunk.remove(chunkKey);
        unsubscribe();
        return false;
      }
      if (_liveSubscriptionsByChunk[chunkKey] != subscription) {
        unsubscribe();
        return true;
      }
    } on _StaleChannelRefresh {
      rethrow;
    } catch (error) {
      if (_liveSubscriptionsByChunk[chunkKey] == subscription) {
        _liveSubscriptionsByChunk.remove(chunkKey);
      }
      if (!_lifecycleRef.mounted) return false;
      debugPrint(
        '[ChannelsNotifier] live subscription failed for '
        '${chunk.length} channels: $error',
      );
    }
    return true;
  }

  /// Notes that the list may have changed and schedules a read: after
  /// [ChannelsNotifier._listReadQuiet] without another such note, and no
  /// later than [ChannelsNotifier._listReadMaxWait] after the first, so a
  /// burst of changes costs one read. However busy the relay is, reads start
  /// at least [ChannelsNotifier._listReadMinGap] apart.
  void _noteListChanged() {
    _channelListChanges++;
    _scheduleListRead();
  }

  /// Retries a failed or partial unread catch-up. The list read reinstalls
  /// live coverage, and the catch-up runs again after it.
  void _retryUnreadCatchUp() {
    _channelListChanges++;
    _readListIfChanged();
  }

  void _scheduleListRead() {
    if (_listReadGap != null) {
      _listReadDueAfterGap = true;
      return;
    }
    _listReadQuietTimer?.cancel();
    _listReadQuietTimer = Timer(
      ChannelsNotifier._listReadQuiet,
      _readListIfChanged,
    );
    _listReadDeadline ??= Timer(
      ChannelsNotifier._listReadMaxWait,
      _readListIfChanged,
    );
  }

  void _readListIfChanged() {
    _listReadQuietTimer?.cancel();
    _listReadQuietTimer = null;
    _listReadDeadline?.cancel();
    _listReadDeadline = null;
    if (!_lifecycleRef.mounted) return;
    _noteReopenedSubscriptions();
    // Resume and reconnect read the list anyway, so a read that comes due
    // in the background or offline is left to them.
    if (_lifecycleRef.read(appLifecycleProvider) != AppLifecycleState.resumed) {
      return;
    }
    // A retry can come due inside the gap after the read it retries.
    if (_listReadGap != null) {
      _listReadDueAfterGap = true;
      return;
    }
    if (_needsSnapshotRevalidation ||
        _channelListChanges != _channelListChangesRead) {
      _listReadGap?.cancel();
      _listReadGap = Timer(ChannelsNotifier._listReadMinGap, () {
        _listReadGap = null;
        if (!_listReadDueAfterGap) return;
        _listReadDueAfterGap = false;
        _readListIfChanged();
      });
      unawaited(_refreshInBackground(queueIfBusy: true));
    }
  }

  void _cancelListRead() {
    _listReadQuietTimer?.cancel();
    _listReadQuietTimer = null;
    _listReadDeadline?.cancel();
    _listReadDeadline = null;
    _listReadGap?.cancel();
    _listReadGap = null;
    _listReadDueAfterGap = false;
    _listReadRetry.cancel();
    _unreadCatchUpRetry.cancel();
  }

  Future<void> _refreshInBackground({bool queueIfBusy = false}) async {
    if (_lifecycleRef.read(relaySessionProvider).status !=
        SessionStatus.connected) {
      return;
    }
    if (_backgroundRefreshInFlight) {
      if (queueIfBusy) _backgroundRefreshQueued = true;
      return;
    }
    final generation = _buildGeneration;
    _backgroundRefreshInFlight = true;
    try {
      if (_needsSnapshotRevalidation) {
        await refresh();
      } else {
        await _readListInBackground();
      }
    } finally {
      if (generation == _buildGeneration) {
        _backgroundRefreshInFlight = false;
        final queued = _backgroundRefreshQueued;
        _backgroundRefreshQueued = false;
        if (!_needsSnapshotRevalidation &&
            _channelListChangesRead >= _channelListChanges) {
          // This read covered everything, including any read queued behind
          // it, so the backoff starts over.
          _listReadRetry.cancel();
        } else if (queued) {
          // The queued read still waits out the gap after the read before it.
          _readListIfChanged();
        } else {
          // The read failed or was incomplete. Nothing else would retry it
          // before the next change, resume or reconnect.
          _listReadRetry.schedule(_readListIfChanged);
        }
      }
    }
  }

  void _handleLiveChunkClosed(String chunkKey, int generation, String message) {
    if (!_lifecycleRef.mounted) return;
    final subscription = _liveSubscriptionsByChunk[chunkKey];
    if (subscription == null || subscription.generation != generation) return;
    _liveSubscriptionsByChunk.remove(chunkKey);
    // RelaySession calls onClosed only for terminal admission failures. Its
    // transient/rate-limit retries have their own backoff. Retry only this
    // REQ, with backoff, rather than a fresh membership/history read, and
    // only when the relay was short of capacity: a refusal for any other
    // reason waits for the next connection.
    if (chunkChannelIdsForLiveSubscriptions(
      _desiredLiveChannelIds,
    ).any((chunk) => _liveChunkKey(chunk) == chunkKey)) {
      if (_isRelayCapacityClose(message)) {
        _terminallyClosedLiveChunks.add(chunkKey);
        _retryClosedLiveChunks();
      } else {
        _refusedLiveChunks.add(chunkKey);
      }
    }
    debugPrint(
      '[ChannelsNotifier] live subscription closed by relay: $message',
    );
  }

  /// Reopens the chunks the relay closed, after a backoff. Events sent while
  /// a chunk was closed are not replayed, so reopening one schedules a read
  /// (see [_noteReopenedSubscriptions]).
  void _retryClosedLiveChunks() {
    _liveChunksRetry.schedule(() {
      _liveSubscriptionQueue = _liveSubscriptionQueue
          .then((_) => _reopenClosedLiveChunks())
          .catchError((Object error, StackTrace stack) {
            debugPrint(
              '[ChannelsNotifier] live subscription retry failed: '
              '$error\n$stack',
            );
          });
    });
  }

  Future<void> _reopenClosedLiveChunks() async {
    final fence = _liveFence;
    final relayBaseUrl = _subscriptionRelayBaseUrl;
    // Disconnected: the reconnect's list read subscribes them again.
    if (!_lifecycleRef.mounted ||
        fence == null ||
        relayBaseUrl == null ||
        _lifecycleRef.read(relaySessionProvider).status !=
            SessionStatus.connected) {
      return;
    }
    final desiredChunks = chunkChannelIdsForLiveSubscriptions(
      _desiredLiveChannelIds,
    );
    _terminallyClosedLiveChunks.retainAll(desiredChunks.map(_liveChunkKey));
    // A list read in flight skips closed chunks, so try again after it.
    if (!fence.isCurrent) {
      if (_terminallyClosedLiveChunks.isNotEmpty) _retryClosedLiveChunks();
      return;
    }
    final subscriptionVersion = _subscriptionVersion;
    for (final chunk in desiredChunks) {
      final chunkKey = _liveChunkKey(chunk);
      if (!_terminallyClosedLiveChunks.remove(chunkKey)) continue;
      var proceed = false;
      try {
        proceed = await _subscribeLiveChunk(
          chunk,
          relayBaseUrl,
          subscriptionVersion,
          fence,
        );
      } on _StaleChannelRefresh {
        // Leave it for the next attempt below.
      }
      if (_liveSubscriptionsByChunk.containsKey(chunkKey)) {
        _reopenedLiveChunks.add(chunkKey);
      } else if (_lifecycleRef.mounted &&
          !_refusedLiveChunks.contains(chunkKey)) {
        _terminallyClosedLiveChunks.add(chunkKey);
      }
      if (!proceed) break;
    }
    if (!_lifecycleRef.mounted) return;
    final reopened = _reopenedLiveChunks.isNotEmpty;
    if (reopened) _scheduleListRead();
    if (_terminallyClosedLiveChunks.isNotEmpty) {
      _retryClosedLiveChunks();
    } else if (reopened) {
      _liveChunksRetry.opened();
    }
  }

  /// When a read comes due, a subscription reopened since the last one that
  /// is still up counts as a change, because what it missed while down is
  /// not replayed. One the relay has closed again is retried, and its next
  /// reopening is checked the same way, so a relay that admits a REQ and then
  /// closes it costs no reads.
  void _noteReopenedSubscriptions() {
    if (_listNoticesReopened &&
        _listNoticesUnsubscribe != null &&
        !_listNoticesInterrupted) {
      _channelListChanges++;
    }
    _listNoticesReopened = false;
    if (_reopenedLiveChunks.any(
      (key) => _liveSubscriptionsByChunk[key]?.interrupted == false,
    )) {
      _channelListChanges++;
    }
    _reopenedLiveChunks.clear();
  }

  void _removeUndesiredLiveChunks() {
    final desiredChunks = chunkChannelIdsForLiveSubscriptions(
      _desiredLiveChannelIds,
    );
    final desiredKeys = desiredChunks.map(_liveChunkKey).toSet();
    final retainedKeys = <String>{...desiredKeys};
    final uncoveredIds = <String>{};
    for (final chunk in desiredChunks) {
      final key = _liveChunkKey(chunk);
      if (!_liveSubscriptionsByChunk.containsKey(key)) {
        uncoveredIds.addAll(chunk);
      }
    }
    final obsolete = _liveSubscriptionsByChunk.entries
        .where((entry) => !desiredKeys.contains(entry.key))
        .toList();
    while (uncoveredIds.isNotEmpty) {
      MapEntry<String, _LiveChunkSubscription>? best;
      var bestCoverage = 0;
      for (final entry in obsolete) {
        if (retainedKeys.contains(entry.key)) continue;
        final coverage = entry.key
            .split('\u0000')
            .where(uncoveredIds.contains)
            .length;
        if (coverage > bestCoverage) {
          best = entry;
          bestCoverage = coverage;
        }
      }
      if (best == null) break;
      retainedKeys.add(best.key);
      uncoveredIds.removeAll(best.key.split('\u0000'));
    }
    for (final entry in _liveSubscriptionsByChunk.entries.toList()) {
      if (retainedKeys.contains(entry.key)) continue;
      _liveSubscriptionsByChunk.remove(entry.key);
      entry.value.unsubscribe?.call();
    }
  }

  void _clearRetainedLiveChunks() {
    for (final subscription in _liveSubscriptionsByChunk.values) {
      subscription.unsubscribe?.call();
    }
    _liveSubscriptionsByChunk.clear();
  }

  void _clearLiveSubscriptions() {
    _subscriptionVersion++;
    _notifyUnreadReadinessChanged();
    _desiredLiveChannelIds = const {};
    _terminallyClosedLiveChunks.clear();
    _refusedLiveChunks.clear();
    _liveChunksRetry.cancel();
    _reopenedLiveChunks.clear();
    _liveFence = null;
    _clearRetainedLiveChunks();
    _clearChannelListNotices();
    _subscriptionRelayBaseUrl = null;
    _cancelListRead();
  }
}

/// Returns deterministic channel-ID chunks within the relay's live REQ cap.
List<List<String>> chunkChannelIdsForLiveSubscriptions(
  Iterable<String> channelIds,
) {
  final sortedIds = channelIds.toList()..sort();
  return [
    for (
      var start = 0;
      start < sortedIds.length;
      start += _maxLiveChannelsPerSubscription
    )
      sortedIds.sublist(
        start,
        min(start + _maxLiveChannelsPerSubscription, sortedIds.length),
      ),
  ];
}

String _liveChunkKey(List<String> channelIds) => channelIds.join('\u0000');

class _LiveChunkSubscription {
  _LiveChunkSubscription(this.generation);

  final int generation;
  void Function()? unsubscribe;
  // Closed by the relay and being reopened by RelaySession itself.
  bool interrupted = false;
}

/// Retries a failed subscription or read after a jittered backoff that starts
/// at 5s and doubles up to 5 minutes, until it succeeds or is torn down.
class _Backoff {
  static const _baseDelayMs = 5000;
  static const _maxDelayMs = 300000;
  // How long a reopened subscription must stay up to restart the backoff.
  static const _stableAfter = Duration(minutes: 1);
  static final _random = Random();

  Timer? _timer;
  Timer? _stableTimer;
  var _attempts = 0;

  /// Runs [retry] after the next backoff, unless one is already pending.
  void schedule(void Function() retry) {
    _stableTimer?.cancel();
    _stableTimer = null;
    if (_timer != null) return;
    final backoffMs = min(_baseDelayMs << min(_attempts, 6), _maxDelayMs);
    _attempts++;
    _timer = Timer(
      Duration(
        milliseconds: (backoffMs * (0.75 + _random.nextDouble() * 0.5)).round(),
      ),
      () {
        _timer = null;
        retry();
      },
    );
  }

  /// Drops a pending retry and keeps the backoff, for an attempt made sooner.
  void cancelPending() {
    _timer?.cancel();
    _timer = null;
  }

  /// Notes a reopened subscription. Its backoff restarts only once it has
  /// stayed up for a minute, so a relay that admits a REQ and then closes it
  /// keeps backing off.
  void opened() {
    _stableTimer?.cancel();
    _stableTimer = Timer(_stableAfter, () {
      _stableTimer = null;
      _attempts = 0;
    });
  }

  /// Drops a pending retry and restarts the backoff.
  void cancel() {
    cancelPending();
    _stableTimer?.cancel();
    _stableTimer = null;
    _attempts = 0;
  }
}
