import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';

/// In-memory cache of other users' presence.
///
/// Live path: kind:20001 over WebSocket.
/// Snapshot path (entity holon R4 / upstream #4417 spirit): on [track], issue
/// a one-shot HTTP `POST /query` for the latest kind:20001 per author so the
/// phone does not claim "offline" until the next heartbeat.
///
/// Place-aware host proofs (host-agentd) remain Desktop/Remote Agents; mobile
/// uses relay presence status only (online/away/offline) — no host paths.
class PresenceCacheNotifier extends Notifier<Map<String, String>> {
  final Set<String> _tracked = {};
  void Function()? _presenceUnsub;
  int _subscriptionVersion = 0;
  int _snapshotGeneration = 0;

  /// Created_at of the latest applied event per pubkey (live or snapshot).
  final Map<String, int> _latestCreatedAt = {};

  @override
  Map<String, String> build() {
    final sessionState = ref.watch(relaySessionProvider);

    ref.onDispose(() {
      _presenceUnsub?.call();
      _presenceUnsub = null;
    });

    if (sessionState.status == SessionStatus.connected) {
      _subscribePresenceUpdates();
    }

    return {};
  }

  /// Track presence for [pubkeys] and fetch a relay snapshot for new ones.
  void track(List<String> pubkeys) {
    final normalized = pubkeys
        .map((pk) => pk.toLowerCase())
        .where((pk) => pk.isNotEmpty)
        .toList();
    final fresh = <String>[];
    for (final pk in normalized) {
      if (_tracked.add(pk)) {
        fresh.add(pk);
      }
    }
    if (fresh.isEmpty) return;
    unawaited(_fetchPresenceSnapshot(fresh));
  }

  Future<void> _fetchPresenceSnapshot(List<String> pubkeys) async {
    final generation = ++_snapshotGeneration;
    final sessionState = ref.read(relaySessionProvider);
    if (sessionState.status != SessionStatus.connected) return;

    final session = ref.read(relaySessionProvider.notifier);
    try {
      // Relay synthesizes/retains presence via query; authors-scoped 20001.
      final events = await session.queryRelay([
        NostrFilter(
          kinds: [EventKind.presenceUpdate],
          authors: pubkeys,
          limit: pubkeys.length,
        ),
      ]);
      if (generation != _snapshotGeneration) return;
      if (events.isEmpty) return;

      // Latest event per subject (author, or p-tag for relay-synthesized).
      final best = <String, NostrEvent>{};
      for (final event in events) {
        final subject = _presenceSubject(event);
        if (!_tracked.contains(subject)) continue;
        final prev = best[subject];
        if (prev == null || event.createdAt >= prev.createdAt) {
          best[subject] = event;
        }
      }
      if (best.isEmpty) return;

      var changed = false;
      final updated = Map<String, String>.from(state);
      best.forEach((pubkey, event) {
        final status = event.content;
        if (status != 'online' && status != 'away' && status != 'offline') {
          return;
        }
        final prevTs = _latestCreatedAt[pubkey] ?? 0;
        if (event.createdAt < prevTs) return;
        _latestCreatedAt[pubkey] = event.createdAt;
        if (updated[pubkey] == status) return;
        updated[pubkey] = status;
        changed = true;
      });
      if (changed) state = updated;
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
    if (status != 'online' && status != 'away' && status != 'offline') return;

    final prevTs = _latestCreatedAt[pubkey] ?? 0;
    // Live events win over equal/older snapshot; prefer newer created_at.
    if (event.createdAt < prevTs) return;
    _latestCreatedAt[pubkey] = event.createdAt;

    if (state[pubkey] == status) return;
    final updated = Map<String, String>.from(state);
    updated[pubkey] = status;
    state = updated;
  }

  /// Subject of a presence event: self-signed uses author; relay-signed may use p.
  static String _presenceSubject(NostrEvent event) {
    for (final tag in event.tags) {
      if (tag.length >= 2 && tag[0] == 'p' && tag[1].isNotEmpty) {
        return tag[1].toLowerCase();
      }
    }
    return event.pubkey.toLowerCase();
  }
}

final presenceCacheProvider =
    NotifierProvider<PresenceCacheNotifier, Map<String, String>>(
      PresenceCacheNotifier.new,
    );
