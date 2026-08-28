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
      if (_unsubscribersByLiveChunk.containsKey(chunkKey)) continue;
      if (_lifecycleRef.read(relaySessionProvider).status !=
          SessionStatus.connected) {
        return;
      }
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
        );
        if (!_lifecycleRef.mounted || !fence.isCurrent) {
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
          unsubscribe();
          return;
        }
        final replaced = _unsubscribersByLiveChunk[chunkKey];
        if (replaced != null) {
          unsubscribe();
          continue;
        }
        _unsubscribersByLiveChunk[chunkKey] = unsubscribe;
      } on _StaleChannelRefresh {
        rethrow;
      } catch (error) {
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

  void _removeUndesiredLiveChunks() {
    final desiredKeys = chunkChannelIdsForLiveSubscriptions(
      _desiredLiveChannelIds,
    ).map(_liveChunkKey).toSet();
    final coveredIds = {
      for (final key in _unsubscribersByLiveChunk.keys)
        if (desiredKeys.contains(key)) ...key.split('\u0000'),
    };
    for (final entry in _unsubscribersByLiveChunk.entries.toList()) {
      if (desiredKeys.contains(entry.key)) continue;
      // A failed or retired replacement must not take unchanged channels
      // offline. Drop an old chunk only once its still-desired channels have
      // replacement coverage (or none of its channels remain desired).
      final stillNeeded = entry.key
          .split('\u0000')
          .any(
            (id) =>
                _desiredLiveChannelIds.contains(id) && !coveredIds.contains(id),
          );
      if (stillNeeded) continue;
      _unsubscribersByLiveChunk.remove(entry.key);
      entry.value();
    }
  }

  void _clearRetainedLiveChunks() {
    for (final unsubscribe in _unsubscribersByLiveChunk.values) {
      unsubscribe();
    }
    _unsubscribersByLiveChunk.clear();
  }

  void _clearLiveSubscriptions() {
    _subscriptionVersion++;
    _desiredLiveChannelIds = const {};
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
