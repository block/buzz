import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';

/// In-memory cache of other users' presence.
///
/// Live kind:20001 updates arrive over the relay WebSocket. An authenticated
/// `/query` snapshot backfills tracked identities immediately and periodically,
/// so a newly opened or resumed app does not have to wait for the next
/// heartbeat and TTL expiry is eventually reflected as `offline`.
class PresenceCacheNotifier extends Notifier<Map<String, String>> {
  static const _batchDelay = Duration(milliseconds: 50);
  static const _refreshInterval = Duration(seconds: 60);
  static const _presenceTtl = Duration(seconds: 180);

  final Set<String> _tracked = {};
  final Set<String> _pending = {};
  final Map<String, String> _statuses = {};
  final Map<String, int> _lastLiveSequence = {};
  final Map<String, int> _lastStatusCreatedAt = {};
  final Map<String, DateTime> _positiveObservedAt = {};
  Timer? _batchTimer;
  Timer? _refreshTimer;
  void Function()? _presenceUnsub;
  String? _relayIdentity;
  bool _snapshotInFlight = false;
  int _subscriptionVersion = 0;
  int _changeSequence = 0;

  @override
  Map<String, String> build() {
    final relayConfig = ref.watch(relayConfigProvider);
    final relayIdentity =
        '${relayConfig.baseUrl}|${pubkeyFromNsec(relayConfig.nsec) ?? ''}';
    if (_relayIdentity != null && _relayIdentity != relayIdentity) {
      _resetForRelayChange();
    }
    _relayIdentity = relayIdentity;
    final sessionState = ref.watch(relaySessionProvider);

    ref.onDispose(() {
      _subscriptionVersion++;
      _pending.clear();
      _batchTimer?.cancel();
      _batchTimer = null;
      _refreshTimer?.cancel();
      _refreshTimer = null;
      _presenceUnsub?.call();
      _presenceUnsub = null;
    });

    if (sessionState.status == SessionStatus.connected) {
      unawaited(_subscribePresenceUpdates());
    }

    return Map.unmodifiable(_statuses);
  }

  void _resetForRelayChange() {
    _subscriptionVersion++;
    _tracked.clear();
    _pending.clear();
    _statuses.clear();
    _lastLiveSequence.clear();
    _lastStatusCreatedAt.clear();
    _positiveObservedAt.clear();
    _changeSequence = 0;
  }

  /// Track presence for [pubkeys] and backfill newly tracked identities.
  void track(List<String> pubkeys) {
    for (final rawPubkey in pubkeys) {
      final pubkey = rawPubkey.trim().toLowerCase();
      if (pubkey.isEmpty || !_tracked.add(pubkey)) continue;
      _pending.add(pubkey);
    }
    _scheduleSnapshot();
  }

  /// Subscribe before querying so a heartbeat cannot fall into a startup gap.
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
      if (version != _subscriptionVersion) {
        unsub();
        return;
      }
      _presenceUnsub = unsub;
      _pending.addAll(_tracked);
      _scheduleSnapshot();
      _ensureRefreshTimer();
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
    if (!_isPresenceStatus(status)) return;

    final previousCreatedAt = _lastStatusCreatedAt[pubkey];
    if (previousCreatedAt != null && event.createdAt < previousCreatedAt) {
      return;
    }
    _lastStatusCreatedAt[pubkey] = event.createdAt;
    _changeSequence++;
    _lastLiveSequence[pubkey] = _changeSequence;
    _setStatus(pubkey, status);
  }

  void _scheduleSnapshot() {
    if (_pending.isEmpty ||
        _presenceUnsub == null ||
        _batchTimer != null ||
        _snapshotInFlight) {
      return;
    }
    _batchTimer = Timer(_batchDelay, _flushPending);
  }

  void _ensureRefreshTimer() {
    _refreshTimer ??= Timer.periodic(_refreshInterval, (_) {
      _pending.addAll(_tracked);
      _scheduleSnapshot();
    });
  }

  Future<void> _flushPending() async {
    _batchTimer = null;
    if (_pending.isEmpty || _presenceUnsub == null) return;

    final pubkeys = _pending.toList(growable: false);
    _pending.clear();
    _snapshotInFlight = true;
    try {
      await _fetchPresenceSnapshot(pubkeys, _subscriptionVersion);
    } finally {
      _snapshotInFlight = false;
      _scheduleSnapshot();
    }
  }

  Future<void> _fetchPresenceSnapshot(
    List<String> pubkeys,
    int subscriptionVersion,
  ) async {
    final querySequence = _changeSequence;
    try {
      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.queryRelay([
        NostrFilter(
          kinds: const [EventKind.presenceUpdate],
          authors: pubkeys,
          limit: pubkeys.length,
        ),
      ]);
      if (subscriptionVersion != _subscriptionVersion) return;

      final requested = pubkeys.toSet();
      final snapshot = <String, NostrEvent>{};
      for (final event in events) {
        if (event.kind != EventKind.presenceUpdate) continue;
        // `/query` returns relay-signed synthetic events whose trusted p-tag
        // identifies the subject. Live WS events deliberately never use this
        // tag, because an arbitrary publisher could spoof it.
        final subject = event.getTagValue('p')?.toLowerCase();
        if (subject == null || !requested.contains(subject)) continue;
        if (!_isPresenceStatus(event.content)) continue;
        final existing = snapshot[subject];
        if (existing == null || event.createdAt > existing.createdAt) {
          snapshot[subject] = event;
        }
      }

      for (final pubkey in pubkeys) {
        if ((_lastLiveSequence[pubkey] ?? 0) > querySequence) continue;
        final event = snapshot[pubkey];
        if (event != null) {
          final previousCreatedAt = _lastStatusCreatedAt[pubkey];
          if (previousCreatedAt != null &&
              event.createdAt < previousCreatedAt) {
            continue;
          }
          _lastStatusCreatedAt[pubkey] = event.createdAt;
          _setStatus(pubkey, event.content);
          continue;
        }

        // Redis is updated before the live heartbeat is fanned out. A missing
        // key can therefore mean expiry, but the relay also deliberately
        // degrades Redis read failures to an empty snapshot. Preserve a fresh
        // positive observation for one relay TTL so a transient read failure
        // cannot make an online DM flash offline.
        final observedAt = _positiveObservedAt[pubkey];
        if (observedAt != null &&
            DateTime.now().difference(observedAt) < _presenceTtl) {
          continue;
        }
        _setStatus(pubkey, 'offline');
      }
    } catch (error) {
      debugPrint('[PresenceCacheNotifier] presence snapshot failed: $error');
    }
  }

  bool _isPresenceStatus(String status) =>
      status == 'online' || status == 'away' || status == 'offline';

  void _setStatus(String pubkey, String status) {
    if (status == 'online' || status == 'away') {
      _positiveObservedAt[pubkey] = DateTime.now();
    } else {
      _positiveObservedAt.remove(pubkey);
    }
    if (_statuses[pubkey] == status) return;
    _statuses[pubkey] = status;
    state = Map.unmodifiable(_statuses);
  }
}

final presenceCacheProvider =
    NotifierProvider<PresenceCacheNotifier, Map<String, String>>(
      PresenceCacheNotifier.new,
    );
