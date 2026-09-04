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
  /// relay's aggregate explicit-`#h` limit. Also starts a 60s WS backstop poll
  /// to reconcile membership changes without downloading the global directory.
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

    final session = _lifecycleRef.read(relaySessionProvider.notifier);
    for (final chunk in desiredChunks) {
      final chunkKey = _liveChunkKey(chunk);
      if (_liveSubscriptionsByChunk.containsKey(chunkKey) ||
          _terminallyClosedLiveChunks.contains(chunkKey)) {
        continue;
      }
      if (_lifecycleRef.read(relaySessionProvider).status !=
          SessionStatus.connected) {
        return;
      }
      final generation = ++_nextLiveChunkGeneration;
      final subscription = _LiveChunkSubscription(generation);
      _liveSubscriptionsByChunk[chunkKey] = subscription;
      try {
        final unsubscribe = await session.subscribe(
          NostrFilter(
            kinds: EventKind.channelEventKinds,
            tags: {'#h': chunk},
            limit: 0,
          ),
          (event) {
            // Superseded chunks can overlap during replacement or recovery.
            // Deliver only the current scope's still-desired channels; duplicate
            // events are already idempotent in the unread/timestamp stores.
            if (!_lifecycleRef.mounted ||
                _refreshCoordinator.currentScope() != fence.scope ||
                !_desiredLiveChannelIds.contains(event.channelId)) {
              return;
            }
            _handleLiveEvent(event);
          },
          onClosed: (message) =>
              _handleLiveChunkClosed(chunkKey, generation, message),
        );
        subscription.unsubscribe = unsubscribe;
        if (!_lifecycleRef.mounted || !fence.isCurrent) {
          _liveSubscriptionsByChunk.remove(chunkKey);
          unsubscribe();
          if (!fence.isCurrent) throw const _StaleChannelRefresh();
          return;
        }
        if (subscriptionVersion != _subscriptionVersion ||
            _lifecycleRef.read(relaySessionProvider).status !=
                SessionStatus.connected ||
            _lifecycleRef.read(relayConfigProvider).baseUrl != relayBaseUrl ||
            _subscriptionRelayBaseUrl != relayBaseUrl ||
            !chunk.every(_desiredLiveChannelIds.contains)) {
          _liveSubscriptionsByChunk.remove(chunkKey);
          unsubscribe();
          return;
        }
        if (_liveSubscriptionsByChunk[chunkKey] != subscription) {
          unsubscribe();
          continue;
        }
      } on _StaleChannelRefresh {
        rethrow;
      } catch (error) {
        if (_liveSubscriptionsByChunk[chunkKey] == subscription) {
          _liveSubscriptionsByChunk.remove(chunkKey);
        }
        if (!_lifecycleRef.mounted) return;
        debugPrint(
          '[ChannelsNotifier] live subscription failed for '
          '${chunk.length} channels: $error',
        );
      }
    }

    if (!_lifecycleRef.mounted ||
        _lifecycleRef.read(relaySessionProvider).status !=
            SessionStatus.connected ||
        subscriptionVersion != _subscriptionVersion) {
      return;
    }

    fence.ensureCurrent();
    unawaited(_catchUpUnreadEvents(channels, fence, subscriptionVersion));

    _backstopTimer?.cancel();
    _backstopTimer = Timer.periodic(
      ChannelsNotifier._backstopInterval,
      (_) => _backstopRefresh(),
    );
  }

  void _handleLiveChunkClosed(String chunkKey, int generation, String message) {
    if (!_lifecycleRef.mounted) return;
    final subscription = _liveSubscriptionsByChunk[chunkKey];
    if (subscription == null || subscription.generation != generation) return;
    _liveSubscriptionsByChunk.remove(chunkKey);
    // RelaySession calls onClosed only for terminal admission failures. Its
    // transient/rate-limit retries have their own backoff. Do not turn a
    // terminal rejection into a fresh membership/history/REQ loop here.
    if (chunkChannelIdsForLiveSubscriptions(
      _desiredLiveChannelIds,
    ).any((chunk) => _liveChunkKey(chunk) == chunkKey)) {
      _terminallyClosedLiveChunks.add(chunkKey);
    }
    debugPrint(
      '[ChannelsNotifier] live subscription closed by relay: $message',
    );
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
    _desiredLiveChannelIds = const {};
    _terminallyClosedLiveChunks.clear();
    _clearRetainedLiveChunks();
    _subscriptionRelayBaseUrl = null;
    _backstopTimer?.cancel();
    _backstopTimer = null;
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
}
