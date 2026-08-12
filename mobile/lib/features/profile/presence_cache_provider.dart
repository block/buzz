import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';

/// In-memory cache of other users' presence.
///
/// Subscribes to kind:20001 presence events over the relay WebSocket for
/// real-time updates. Hydrates newly tracked users from relay-generated
/// kind:40902 snapshots so the UI does not wait for the next heartbeat.
class PresenceCacheNotifier extends Notifier<Map<String, String>> {
  static const _batchDelay = Duration(milliseconds: 50);

  final Set<String> _tracked = {};
  final Set<String> _pending = {};
  final Map<String, int> _revisions = {};
  Timer? _batchTimer;
  void Function()? _presenceUnsub;
  int _subscriptionVersion = 0;

  @override
  Map<String, String> build() {
    final sessionState = ref.watch(relaySessionProvider);

    ref.onDispose(() {
      _batchTimer?.cancel();
      _batchTimer = null;
      _pending.clear();
      _presenceUnsub?.call();
      _presenceUnsub = null;
    });

    if (sessionState.status == SessionStatus.connected) {
      _subscribePresenceUpdates();
      Future.microtask(_refreshAll);
    }

    return {};
  }

  /// Track presence for [pubkeys].
  void track(List<String> pubkeys) {
    final normalized = pubkeys
        .map((pk) => pk.toLowerCase())
        .where((pk) => pk.isNotEmpty)
        .toList();
    final uncached = normalized
        .where((pk) => !state.containsKey(pk) && !_pending.contains(pk))
        .toList();

    _tracked.addAll(normalized);

    if (uncached.isEmpty) return;
    _pending.addAll(uncached);
    _batchTimer ??= Timer(_batchDelay, _flushPending);
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

  void _handlePresenceEvent(NostrEvent event) {
    final pubkey = event.pubkey.toLowerCase();
    if (!_tracked.contains(pubkey)) return;
    final status = event.content;
    if (!_isValidStatus(status)) return;
    _updatePresence(pubkey, status);
  }

  Future<void> _refreshAll() async {
    if (_tracked.isEmpty) return;
    await _fetchPresence(_tracked.toList());
  }

  Future<void> _flushPending() async {
    _batchTimer = null;
    if (_pending.isEmpty) return;

    final pubkeys = _pending.toList();
    _pending.clear();
    await _fetchPresence(pubkeys);
  }

  Future<void> _fetchPresence(List<String> pubkeys) async {
    final requested = pubkeys.toSet().toList();
    if (requested.isEmpty) return;

    // Relay queries require NIP-98 authentication. A connected-looking dev or
    // test session can still be anonymous, so avoid starting a request that is
    // guaranteed to fail until an identity is available.
    final nsec = ref.read(relayConfigProvider).nsec;
    if (nsec == null || nsec.isEmpty) return;

    final startingRevisions = {
      for (final pubkey in requested) pubkey: _revisions[pubkey] ?? 0,
    };

    try {
      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.queryRelay([
        NostrFilter(
          kinds: const [EventKind.presenceSnapshot],
          authors: requested,
          limit: requested.length,
        ),
      ]);

      final statuses = <String, ({int createdAt, String status})>{};
      for (final event in events) {
        final pubkey = event.getTagValue('p')?.toLowerCase();
        final status = event.content;
        if (pubkey == null ||
            !startingRevisions.containsKey(pubkey) ||
            !_isValidStatus(status)) {
          continue;
        }
        final existing = statuses[pubkey];
        if (existing == null || event.createdAt > existing.createdAt) {
          statuses[pubkey] = (createdAt: event.createdAt, status: status);
        }
      }

      for (final pubkey in requested) {
        // A live update received while this request was in flight is newer
        // than the snapshot and must win.
        if ((_revisions[pubkey] ?? 0) != startingRevisions[pubkey]) continue;
        _updatePresence(pubkey, statuses[pubkey]?.status ?? 'offline');
      }
    } catch (error) {
      debugPrint('[PresenceCacheNotifier] presence snapshot failed: $error');
    }
  }

  void _updatePresence(String pubkey, String status) {
    if (state[pubkey] == status) return;
    final updated = Map<String, String>.from(state);
    updated[pubkey] = status;
    _revisions[pubkey] = (_revisions[pubkey] ?? 0) + 1;
    state = updated;
  }

  bool _isValidStatus(String status) =>
      status == 'online' || status == 'away' || status == 'offline';
}

final presenceCacheProvider =
    NotifierProvider<PresenceCacheNotifier, Map<String, String>>(
      PresenceCacheNotifier.new,
    );
