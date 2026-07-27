import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';

/// In-memory cache of other users' presence.
///
/// Subscribes to kind:20001 presence events over the relay WebSocket for
/// real-time updates and queries the relay's presence snapshot when a pubkey
/// is first tracked. The snapshot closes the gap between an agent publishing
/// online and mobile opening its live subscription.
class PresenceCacheNotifier extends Notifier<Map<String, String>> {
  final Set<String> _tracked = {};
  final Set<String> _pendingSnapshotPubkeys = {};
  void Function()? _presenceUnsub;
  Timer? _snapshotBatchTimer;
  int _subscriptionVersion = 0;

  @override
  Map<String, String> build() {
    final sessionState = ref.watch(relaySessionProvider);

    ref.onDispose(() {
      _presenceUnsub?.call();
      _presenceUnsub = null;
      _snapshotBatchTimer?.cancel();
      _snapshotBatchTimer = null;
    });

    if (sessionState.status == SessionStatus.connected) {
      _subscribePresenceUpdates();
      _queuePresenceSnapshot(_tracked);
    }

    return {};
  }

  /// Track presence for [pubkeys].
  ///
  /// New pubkeys are resolved in one batched relay snapshot query. Live
  /// kind:20001 events continue to update the cache after that initial read.
  void track(List<String> pubkeys) {
    final normalized = pubkeys.map((pk) => pk.toLowerCase()).toList();
    final newlyTracked = normalized.where(_tracked.add);
    _queuePresenceSnapshot(newlyTracked);
  }

  void _queuePresenceSnapshot(Iterable<String> pubkeys) {
    _pendingSnapshotPubkeys.addAll(pubkeys);
    if (_pendingSnapshotPubkeys.isEmpty || _snapshotBatchTimer != null) return;
    _snapshotBatchTimer = Timer(Duration.zero, () {
      _snapshotBatchTimer = null;
      unawaited(_fetchPresenceSnapshot());
    });
  }

  Future<void> _fetchPresenceSnapshot() async {
    if (_pendingSnapshotPubkeys.isEmpty) return;
    final pubkeys = _pendingSnapshotPubkeys.toList();
    _pendingSnapshotPubkeys.clear();

    try {
      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.queryRelay([
        NostrFilter(
          kinds: const [EventKind.presenceUpdate],
          authors: pubkeys,
          limit: pubkeys.length,
        ),
      ]);
      for (final event in events) {
        _handlePresenceEvent(event, useTaggedSubject: true);
      }
    } catch (error) {
      debugPrint('[PresenceCacheNotifier] presence snapshot failed: $error');
    }
  }

  /// Subscribe to kind:20001 presence events over WebSocket.
  Future<void> _subscribePresenceUpdates() async {
    _presenceUnsub?.call();
    _presenceUnsub = null;
    _subscriptionVersion++;
    final version = _subscriptionVersion;

    final session = ref.read(relaySessionProvider.notifier);
    try {
      final unsub = await session.subscribe(
        const NostrFilter(kinds: [EventKind.presenceUpdate], limit: 0),
        _handlePresenceEvent,
      );
      // Guard: if build() re-fired while we were awaiting, discard this
      // subscription to avoid leaking it.
      if (version != _subscriptionVersion) {
        unsub();
        return;
      }
      _presenceUnsub = unsub;
    } catch (error) {
      debugPrint(
        '[PresenceCacheNotifier] presence subscription failed: $error',
      );
    }
  }

  void _handlePresenceEvent(NostrEvent event, {bool useTaggedSubject = false}) {
    // Relay snapshot events are signed by the relay and identify the queried
    // subject with a p-tag. Live events are self-signed, so their author is the
    // subject and an arbitrary p-tag must not be allowed to impersonate it.
    final taggedSubject = useTaggedSubject ? event.getTagValue('p') : null;
    final pubkey = (taggedSubject ?? event.pubkey).toLowerCase();
    if (!_tracked.contains(pubkey)) return;
    final status = event.content;
    if (status != 'online' && status != 'away' && status != 'offline') return;
    if (state[pubkey] == status) return;
    final updated = Map<String, String>.from(state);
    updated[pubkey] = status;
    state = updated;
  }
}

final presenceCacheProvider =
    NotifierProvider<PresenceCacheNotifier, Map<String, String>>(
      PresenceCacheNotifier.new,
    );
