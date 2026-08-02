import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';

/// In-memory cache of other users' presence.
///
/// Subscribes to kind:20001 presence events over the relay WebSocket for
/// real-time updates and queries the relay's Redis-backed presence snapshot
/// when a pubkey is first tracked. The snapshot closes the race where a newly
/// paired mobile client otherwise shows everyone offline until the next live
/// heartbeat.
class PresenceCacheNotifier extends Notifier<Map<String, String>> {
  static const _refreshInterval = Duration(seconds: 120);

  final Set<String> _tracked = {};
  final Set<String> _pending = {};
  Timer? _batchTimer;
  Timer? _refreshTimer;
  void Function()? _presenceUnsub;
  int _subscriptionVersion = 0;

  @override
  Map<String, String> build() {
    final sessionState = ref.watch(relaySessionProvider);

    ref.onDispose(() {
      _batchTimer?.cancel();
      _batchTimer = null;
      _refreshTimer?.cancel();
      _refreshTimer = null;
      _presenceUnsub?.call();
      _presenceUnsub = null;
    });

    if (sessionState.status == SessionStatus.connected) {
      _subscribePresenceUpdates();
    }

    return {};
  }

  /// Track presence for [pubkeys].
  ///
  /// Newly tracked pubkeys are fetched in a short batch while all tracked
  /// pubkeys continue receiving live updates.
  void track(List<String> pubkeys) {
    final normalized = pubkeys.map((pk) => pk.toLowerCase()).toList();
    final uncached = normalized
        .where((pk) => !state.containsKey(pk) && !_pending.contains(pk))
        .toList();
    _tracked.addAll(normalized);
    _ensureRefreshTimer();
    if (uncached.isEmpty) return;
    _pending.addAll(uncached);
    _batchTimer ??= Timer(const Duration(milliseconds: 50), _flushPending);
  }

  void _ensureRefreshTimer() {
    _refreshTimer ??= Timer.periodic(_refreshInterval, (_) => _refreshAll());
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
    _applyPresence(pubkey, event.content);
  }

  void _applyPresence(String pubkey, String status) {
    if (!_tracked.contains(pubkey)) return;
    if (status != 'online' && status != 'away' && status != 'offline') return;
    if (state[pubkey] == status) return;
    final updated = Map<String, String>.from(state);
    updated[pubkey] = status;
    state = updated;
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
    try {
      final events = await ref.read(relaySessionProvider.notifier).queryRelay([
        NostrFilter(
          kinds: const [EventKind.presenceUpdate],
          authors: pubkeys,
          limit: pubkeys.length,
        ),
      ]);
      final statuses = <String, String>{
        for (final pk in pubkeys) pk: 'offline',
      };
      for (final event in events) {
        // Snapshot events are relay-signed and name the actual subject in p.
        final subject = event.getTagValue('p')?.toLowerCase();
        if (subject == null || !pubkeys.contains(subject)) continue;
        if (event.content == 'online' ||
            event.content == 'away' ||
            event.content == 'offline') {
          statuses[subject] = event.content;
        }
      }
      final updated = Map<String, String>.from(state)..addAll(statuses);
      state = updated;
    } catch (error) {
      debugPrint('[PresenceCacheNotifier] presence snapshot failed: $error');
    }
  }
}

final presenceCacheProvider =
    NotifierProvider<PresenceCacheNotifier, Map<String, String>>(
      PresenceCacheNotifier.new,
    );
