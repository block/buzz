import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';

/// In-memory cache of other users' presence.
///
/// Subscribes to kind:20001 presence events over the relay WebSocket for
/// real-time updates and queries the relay's current presence snapshot when
/// pubkeys are first tracked, the session reconnects, and periodically while
/// connected.
class PresenceCacheNotifier extends Notifier<Map<String, String>> {
  static const _snapshotStaleAfter = Duration(seconds: 30);
  static const _snapshotBatchDelay = Duration(milliseconds: 50);
  static const _defaultRefreshInterval = Duration(seconds: 60);

  /// Creates a presence cache with an optional refresh interval for tests.
  PresenceCacheNotifier({Duration refreshInterval = _defaultRefreshInterval})
    : _refreshInterval = refreshInterval;

  final Set<String> _tracked = {};
  final Set<String> _pendingSnapshot = {};
  final Map<String, int> _inFlightSnapshotVersion = {};
  final Map<String, int> _latestSnapshotCreatedAt = {};
  final Map<String, int> _liveArrivalRevision = {};
  final Map<String, DateTime> _snapshotFetchedAt = {};
  final Duration _refreshInterval;
  Timer? _snapshotTimer;
  Timer? _refreshTimer;
  void Function()? _presenceUnsub;
  int _subscriptionVersion = 0;
  int _snapshotVersion = 0;

  @override
  Map<String, String> build() {
    final sessionState = ref.watch(relaySessionProvider);
    _snapshotVersion++;
    _latestSnapshotCreatedAt.clear();
    _liveArrivalRevision.clear();
    _snapshotTimer?.cancel();
    _snapshotTimer = null;
    _refreshTimer?.cancel();
    _refreshTimer = null;
    _pendingSnapshot.clear();
    _inFlightSnapshotVersion.clear();

    ref.onDispose(() {
      _snapshotTimer?.cancel();
      _snapshotTimer = null;
      _refreshTimer?.cancel();
      _refreshTimer = null;
      _presenceUnsub?.call();
      _presenceUnsub = null;
      _snapshotVersion++;
    });

    if (sessionState.status == SessionStatus.connected) {
      _subscribePresenceUpdates();
      _scheduleSnapshot(_tracked, force: true);
      _ensureRefreshTimer();
    }

    return {};
  }

  /// Track presence for [pubkeys] and fetch their current relay snapshot.
  void track(List<String> pubkeys) {
    final normalized = pubkeys
        .map((pk) => pk.toLowerCase())
        .where((pk) => pk.isNotEmpty)
        .toSet();
    _tracked.addAll(normalized);
    if (ref.read(relaySessionProvider).status == SessionStatus.connected) {
      _scheduleSnapshot(normalized);
      _ensureRefreshTimer();
    }
  }

  void _ensureRefreshTimer() {
    if (_tracked.isEmpty || _refreshTimer != null) return;
    _refreshTimer = Timer.periodic(_refreshInterval, (_) {
      if (ref.read(relaySessionProvider).status == SessionStatus.connected) {
        _scheduleSnapshot(_tracked, force: true);
      }
    });
  }

  void _scheduleSnapshot(Iterable<String> pubkeys, {bool force = false}) {
    final now = DateTime.now();
    for (final pubkey in pubkeys) {
      if (_inFlightSnapshotVersion[pubkey] == _snapshotVersion) continue;
      final fetchedAt = _snapshotFetchedAt[pubkey];
      if (force ||
          fetchedAt == null ||
          now.difference(fetchedAt) >= _snapshotStaleAfter) {
        _pendingSnapshot.add(pubkey);
      }
    }
    if (_pendingSnapshot.isEmpty) return;
    _snapshotTimer ??= Timer(_snapshotBatchDelay, _flushPendingSnapshot);
  }

  Future<void> _flushPendingSnapshot() async {
    _snapshotTimer = null;
    final pubkeys = _pendingSnapshot.toList();
    _pendingSnapshot.clear();
    if (pubkeys.isEmpty) return;
    final requested = pubkeys.toSet();
    final version = _snapshotVersion;
    for (final pubkey in requested) {
      _inFlightSnapshotVersion[pubkey] = version;
    }
    final session = ref.read(relaySessionProvider.notifier);
    final liveRevisionBeforeQuery = {
      for (final pubkey in pubkeys) pubkey: _liveArrivalRevision[pubkey] ?? 0,
    };

    try {
      final events = await session.queryRelay([
        NostrFilter(kinds: const [EventKind.presenceUpdate], authors: pubkeys),
      ]);
      if (version != _snapshotVersion) return;

      final returned = <String>{};
      final updated = Map<String, String>.from(state);
      var changed = false;
      for (final event in events) {
        final taggedPubkey = event.getTagValue('p');
        final pubkey =
            (taggedPubkey == null || taggedPubkey.isEmpty
                    ? event.pubkey
                    : taggedPubkey)
                .toLowerCase();
        if (!requested.contains(pubkey) || !_isPresenceStatus(event.content)) {
          continue;
        }
        returned.add(pubkey);
        changed = _mergeSnapshotEvent(updated, pubkey, event) || changed;
      }

      final fetchedAt = DateTime.now();
      for (final pubkey in pubkeys) {
        _snapshotFetchedAt[pubkey] = fetchedAt;
        if (returned.contains(pubkey)) continue;
        // Do not let an absence from a snapshot overwrite a live event that
        // arrived while the HTTP request was in flight.
        if ((_liveArrivalRevision[pubkey] ?? 0) !=
            liveRevisionBeforeQuery[pubkey]) {
          continue;
        }
        if (updated[pubkey] == 'offline') continue;
        updated[pubkey] = 'offline';
        changed = true;
      }
      if (changed) state = updated;
    } catch (error) {
      if (version == _snapshotVersion) {
        // Bound rebuild-triggered retries. Reconnects and the periodic
        // backstop still bypass this freshness window with force: true.
        final failedAt = DateTime.now();
        for (final pubkey in pubkeys) {
          _snapshotFetchedAt[pubkey] = failedAt;
        }
      }
      debugPrint('[PresenceCacheNotifier] presence snapshot failed: $error');
    } finally {
      for (final pubkey in requested) {
        if (_inFlightSnapshotVersion[pubkey] == version) {
          _inFlightSnapshotVersion.remove(pubkey);
        }
      }
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

  void _handlePresenceEvent(NostrEvent event) {
    final pubkey = event.pubkey.toLowerCase();
    if (!_tracked.contains(pubkey) || !_isPresenceStatus(event.content)) {
      return;
    }
    _liveArrivalRevision[pubkey] = (_liveArrivalRevision[pubkey] ?? 0) + 1;
    _applyPresenceStatus(pubkey, event.content);
  }

  bool _mergeSnapshotEvent(
    Map<String, String> updated,
    String pubkey,
    NostrEvent event,
  ) {
    if (!_tracked.contains(pubkey)) return false;
    final status = event.content;
    if (!_isPresenceStatus(status)) return false;
    final latestCreatedAt = _latestSnapshotCreatedAt[pubkey];
    if (latestCreatedAt != null && latestCreatedAt >= event.createdAt) {
      return false;
    }
    _latestSnapshotCreatedAt[pubkey] = event.createdAt;
    if (updated[pubkey] == status) return false;
    updated[pubkey] = status;
    return true;
  }

  void _applyPresenceStatus(String pubkey, String status) {
    if (state[pubkey] == status) return;
    final updated = Map<String, String>.from(state);
    updated[pubkey] = status;
    state = updated;
  }

  bool _isPresenceStatus(String status) =>
      status == 'online' || status == 'away' || status == 'offline';
}

final presenceCacheProvider =
    NotifierProvider<PresenceCacheNotifier, Map<String, String>>(
      PresenceCacheNotifier.new,
    );
