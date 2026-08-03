import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';

/// In-memory cache of other users' presence.
///
/// Subscribes to kind:20001 presence events over the relay WebSocket for
/// real-time updates. There is no longer a REST backstop — agents that
/// publish presence purely over WS are fine, and TTL expiry will be handled
/// by the relay-side `presence:true` filter extension when that lands.
class PresenceCacheNotifier extends Notifier<Map<String, String>> {
  static const _backstopInterval = Duration(seconds: 60);

  final Set<String> _tracked = {};
  void Function()? _presenceUnsub;
  Timer? _backstopTimer;
  int _subscriptionVersion = 0;

  @override
  Map<String, String> build() {
    final sessionState = ref.watch(relaySessionProvider);

    ref.onDispose(() {
      _presenceUnsub?.call();
      _presenceUnsub = null;
      _backstopTimer?.cancel();
      _backstopTimer = null;
    });

    if (sessionState.status == SessionStatus.connected) {
      _subscribePresenceUpdates();
      _refreshTrackedPresence();
      _backstopTimer?.cancel();
      _backstopTimer = Timer.periodic(
        _backstopInterval,
        (_) => _refreshTrackedPresence(),
      );
    }

    return {};
  }

  /// Track presence for [pubkeys].
  ///
  void track(List<String> pubkeys) {
    final normalized = pubkeys.map((pk) => pk.toLowerCase()).toList();
    var changed = false;
    for (final pubkey in normalized) {
      changed = _tracked.add(pubkey) || changed;
    }
    if (changed &&
        ref.read(relaySessionProvider).status == SessionStatus.connected) {
      _refreshTrackedPresence();
    }
  }

  /// Hydrate the cache from the relay's Redis-backed presence snapshot.
  ///
  /// Presence events are ephemeral, so a client that opens after an agent's
  /// heartbeat cannot rely on the live subscription alone.
  Future<void> _refreshTrackedPresence() async {
    if (_tracked.isEmpty) return;
    try {
      final events = await ref
          .read(relaySessionProvider.notifier)
          .queryRelay([
            NostrFilter(
              kinds: const [EventKind.presenceUpdate],
              authors: _tracked.toList(),
              limit: _tracked.length,
            ),
          ]);
      for (final event in events) {
        _handlePresenceEvent(event);
      }
    } catch (error) {
      debugPrint('[PresenceCacheNotifier] presence refresh failed: $error');
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
    // Relay-synthesized snapshots are relay-signed and identify the actual
    // subject with a p-tag. Live events are self-signed by their subject.
    final taggedSubject = event.tags
        .where((tag) => tag.length >= 2 && tag[0] == 'p')
        .map((tag) => tag[1])
        .firstOrNull;
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
