import 'dart:async';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';

/// In-memory cache of other users' presence.
///
/// Subscribes to kind:20001 presence events over the relay WebSocket for
/// real-time updates, then seeds newly tracked pubkeys from the relay's
/// synthesized current-presence snapshots over `POST /query`.
class PresenceCacheNotifier extends Notifier<Map<String, String>> {
  static const _maxRetryDelayMs = 30000;
  static const _subscriptionStabilityWindow = Duration(seconds: 30);

  /// Creates a presence cache.
  ///
  /// [subscriptionRetryBaseDelay] is configurable so retry behavior can be
  /// exercised deterministically in tests. [snapshotRetryBaseDelay] controls
  /// the equivalent retry path for one-shot snapshot failures.
  PresenceCacheNotifier({
    Duration subscriptionRetryBaseDelay = const Duration(seconds: 1),
    Duration snapshotRetryBaseDelay = const Duration(seconds: 1),
  }) : _subscriptionRetryBaseDelay = subscriptionRetryBaseDelay,
       _snapshotRetryBaseDelay = snapshotRetryBaseDelay;

  final Duration _subscriptionRetryBaseDelay;
  final Duration _snapshotRetryBaseDelay;
  final Set<String> _tracked = {};
  final Set<String> _snapshotRequested = {};
  final Map<String, int> _liveUpdateVersions = {};
  void Function()? _presenceUnsub;
  Future<bool>? _presenceSubscriptionReady;
  Timer? _subscriptionRetryTimer;
  Timer? _subscriptionStabilityTimer;
  Timer? _snapshotRetryTimer;
  int _subscriptionVersion = 0;
  int _subscriptionRetryAttempt = 0;
  int _snapshotRetryAttempt = 0;
  bool _disposed = false;

  @override
  Map<String, String> build() {
    _disposed = false;
    final sessionState = ref.watch(relaySessionProvider);

    ref.onDispose(() {
      _disposed = true;
      _stopPresenceSubscription(clearSnapshots: false);
    });

    if (sessionState.status == SessionStatus.connected) {
      _startPresenceSubscription(resetBackoff: true, resetSnapshots: true);
    } else {
      _stopPresenceSubscription(clearSnapshots: true);
    }

    return {};
  }

  /// Track presence for [pubkeys].
  ///
  /// The live subscription is established before a one-shot snapshot query is
  /// issued, closing the gap where an update could land between the seed and
  /// subscription. Duplicate calls are coalesced; failed requests retry with
  /// capped backoff.
  void track(List<String> pubkeys) {
    final normalized = pubkeys
        .map((pk) => pk.trim().toLowerCase())
        .where((pk) => pk.isNotEmpty);
    _tracked.addAll(normalized);
    unawaited(_fetchPendingSnapshots());
  }

  void _startPresenceSubscription({
    bool resetBackoff = false,
    bool resetSnapshots = false,
  }) {
    if (_disposed ||
        ref.read(relaySessionProvider).status != SessionStatus.connected) {
      return;
    }
    _subscriptionRetryTimer?.cancel();
    _subscriptionRetryTimer = null;
    _subscriptionStabilityTimer?.cancel();
    _subscriptionStabilityTimer = null;
    if (resetBackoff) _subscriptionRetryAttempt = 0;
    if (resetSnapshots) _snapshotRequested.clear();

    final ready = _subscribePresenceUpdates();
    _presenceSubscriptionReady = ready;
    unawaited(
      ready.then((subscribed) {
        if (_disposed || !identical(ready, _presenceSubscriptionReady)) return;
        if (subscribed) {
          _scheduleSubscriptionStabilityReset(ready);
          unawaited(_fetchPendingSnapshots());
        } else {
          _scheduleSubscriptionRetry();
        }
      }),
    );
  }

  void _stopPresenceSubscription({required bool clearSnapshots}) {
    _subscriptionRetryTimer?.cancel();
    _subscriptionRetryTimer = null;
    _subscriptionStabilityTimer?.cancel();
    _subscriptionStabilityTimer = null;
    _snapshotRetryTimer?.cancel();
    _snapshotRetryTimer = null;
    _presenceUnsub?.call();
    _presenceUnsub = null;
    _presenceSubscriptionReady = null;
    _subscriptionVersion++;
    _subscriptionRetryAttempt = 0;
    _snapshotRetryAttempt = 0;
    if (clearSnapshots) _snapshotRequested.clear();
  }

  /// Subscribe to kind:20001 presence events over WebSocket.
  Future<bool> _subscribePresenceUpdates() async {
    _presenceUnsub?.call();
    _presenceUnsub = null;
    _subscriptionVersion++;
    final version = _subscriptionVersion;

    try {
      if (_disposed) return false;
      final session = ref.read(relaySessionProvider.notifier);
      final unsub = await session.subscribe(
        const NostrFilter(kinds: [EventKind.presenceUpdate], limit: 0),
        _handlePresenceEvent,
        onClosed: (message) => _handlePresenceClosed(version, message),
      );
      // Guard: if build() re-fired while we were awaiting, discard this
      // subscription to avoid leaking it.
      if (_disposed || version != _subscriptionVersion) {
        unsub();
        return false;
      }
      _presenceUnsub = unsub;
      return true;
    } catch (error) {
      debugPrint(
        '[PresenceCacheNotifier] presence subscription failed: $error',
      );
      return false;
    }
  }

  void _handlePresenceClosed(int version, String message) {
    if (_disposed || version != _subscriptionVersion) return;
    _subscriptionStabilityTimer?.cancel();
    _subscriptionStabilityTimer = null;
    _snapshotRetryTimer?.cancel();
    _snapshotRetryTimer = null;
    _presenceUnsub = null;
    _presenceSubscriptionReady = null;
    _subscriptionVersion++;
    _snapshotRetryAttempt = 0;
    _snapshotRequested.clear();
    _clearTrackedStatuses();
    debugPrint(
      '[PresenceCacheNotifier] presence subscription closed: $message',
    );
    _scheduleSubscriptionRetry();
  }

  void _scheduleSubscriptionRetry() {
    if (_disposed ||
        _subscriptionRetryTimer != null ||
        ref.read(relaySessionProvider).status != SessionStatus.connected) {
      return;
    }

    final exponent = min(_subscriptionRetryAttempt, 10);
    final delayMs = min(
      _subscriptionRetryBaseDelay.inMilliseconds * (1 << exponent),
      _maxRetryDelayMs,
    );
    _subscriptionRetryAttempt++;
    _subscriptionRetryTimer = Timer(Duration(milliseconds: delayMs), () {
      _subscriptionRetryTimer = null;
      if (_disposed) return;
      _startPresenceSubscription();
    });
  }

  void _scheduleSubscriptionStabilityReset(Future<bool> ready) {
    _subscriptionStabilityTimer?.cancel();
    _subscriptionStabilityTimer = Timer(_subscriptionStabilityWindow, () {
      _subscriptionStabilityTimer = null;
      if (_disposed || !identical(ready, _presenceSubscriptionReady)) return;
      _subscriptionRetryAttempt = 0;
    });
  }

  void _scheduleSnapshotRetry() {
    if (_disposed ||
        _snapshotRetryTimer != null ||
        ref.read(relaySessionProvider).status != SessionStatus.connected) {
      return;
    }

    final exponent = min(_snapshotRetryAttempt, 10);
    final delayMs = min(
      _snapshotRetryBaseDelay.inMilliseconds * (1 << exponent),
      _maxRetryDelayMs,
    );
    _snapshotRetryAttempt++;
    _snapshotRetryTimer = Timer(Duration(milliseconds: delayMs), () {
      _snapshotRetryTimer = null;
      if (_disposed) return;
      unawaited(_fetchPendingSnapshots());
    });
  }

  void _handlePresenceEvent(NostrEvent event) {
    if (_disposed) return;
    final pubkey = event.pubkey.toLowerCase();
    if (!_tracked.contains(pubkey)) return;
    final status = event.content;
    if (status != 'online' && status != 'away' && status != 'offline') return;

    // Advance even for a no-op status so a snapshot request already in flight
    // cannot overwrite a newer live event with stale relay state.
    _liveUpdateVersions[pubkey] = (_liveUpdateVersions[pubkey] ?? 0) + 1;
    _applyStatus(pubkey, status);
  }

  Future<void> _fetchPendingSnapshots() async {
    if (_disposed) return;
    final ready = _presenceSubscriptionReady;
    if (ready == null || !await ready) return;
    if (_disposed || !identical(ready, _presenceSubscriptionReady)) return;
    final subscriptionVersion = _subscriptionVersion;

    final pubkeys = _tracked.difference(_snapshotRequested).toList()..sort();
    if (pubkeys.isEmpty) return;

    _snapshotRetryTimer?.cancel();
    _snapshotRetryTimer = null;

    // Claim before yielding to queryRelay so concurrent track() calls cannot
    // request the same pubkey twice.
    _snapshotRequested.addAll(pubkeys);
    final liveVersions = {
      for (final pubkey in pubkeys) pubkey: _liveUpdateVersions[pubkey] ?? 0,
    };

    try {
      if (_disposed) return;
      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.queryRelay([
        NostrFilter(
          kinds: const [EventKind.presenceUpdate],
          authors: pubkeys,
          limit: pubkeys.length,
        ),
      ]);
      if (_disposed || subscriptionVersion != _subscriptionVersion) return;

      final resolvedPubkeys = <String>{};
      for (final event in events) {
        if (event.kind != EventKind.presenceUpdate) continue;
        final pubkey = (event.getTagValue('p') ?? event.pubkey)
            .trim()
            .toLowerCase();
        if (!liveVersions.containsKey(pubkey) || !_tracked.contains(pubkey)) {
          continue;
        }
        if ((_liveUpdateVersions[pubkey] ?? 0) != liveVersions[pubkey]) {
          continue;
        }
        final status = event.content;
        if (status != 'online' && status != 'away' && status != 'offline') {
          continue;
        }
        resolvedPubkeys.add(pubkey);
        _applyStatus(pubkey, status);
      }

      // Redis omits expired/absent presence. Once the query succeeds, that
      // absence is the authoritative offline state unless a live event arrived
      // while the request was in flight.
      for (final pubkey in pubkeys) {
        if (resolvedPubkeys.contains(pubkey)) continue;
        if ((_liveUpdateVersions[pubkey] ?? 0) != liveVersions[pubkey]) {
          continue;
        }
        _applyStatus(pubkey, 'offline');
      }
      _snapshotRetryAttempt = 0;
    } catch (error) {
      if (!_disposed && subscriptionVersion == _subscriptionVersion) {
        _snapshotRequested.removeAll(pubkeys);
        debugPrint('[PresenceCacheNotifier] presence snapshot failed: $error');
        _scheduleSnapshotRetry();
      }
    }
  }

  void _clearTrackedStatuses() {
    if (_disposed || !state.keys.any(_tracked.contains)) return;
    final updated = Map<String, String>.from(state)
      ..removeWhere((pubkey, _) => _tracked.contains(pubkey));
    state = updated;
  }

  void _applyStatus(String pubkey, String status) {
    if (_disposed) return;
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
