import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';

/// In-memory cache of other users' presence.
///
/// Subscribes to kind:20001 presence events over the relay WebSocket for
/// real-time updates, and seeds the cache with a one-shot HTTP bridge query
/// (`POST /query`) per newly tracked pubkey — without it, everyone reads as
/// offline until their next heartbeat arrives (up to 60s for agents, or
/// never for users who published before this client connected).
class PresenceCacheNotifier extends Notifier<Map<String, String>> {
  final Set<String> _tracked = {};
  void Function()? _presenceUnsub;
  int _subscriptionVersion = 0;

  @override
  Map<String, String> build() {
    final sessionState = ref.watch(relaySessionProvider);

    ref.onDispose(() {
      _presenceUnsub?.call();
      _presenceUnsub = null;
    });

    if (sessionState.status == SessionStatus.connected) {
      _subscribePresenceUpdates();
      // Re-seed on (re)connect: covers pubkeys tracked while disconnected
      // and state that changed during the gap.
      if (_tracked.isNotEmpty) {
        unawaited(_fetchPresence(_tracked.toList()));
      }
    }

    return {};
  }

  /// Track presence for [pubkeys] and seed their current state once.
  void track(List<String> pubkeys) {
    final normalized = pubkeys.map((pk) => pk.toLowerCase()).toList();
    final unseen = normalized.where((pk) => !_tracked.contains(pk)).toList();
    _tracked.addAll(normalized);
    if (unseen.isEmpty) return;
    unawaited(_fetchPresence(unseen));
  }

  /// One-shot fetch of current presence state via the relay's HTTP bridge.
  ///
  /// The relay synthesizes kind:20001 events from its presence store. Unlike
  /// live events (signed by the subject), synthesized events are relay-signed
  /// with the subject pubkey in the `p` tag — [_presenceSubject] handles both.
  Future<void> _fetchPresence(List<String> pubkeys) async {
    final session = ref.read(relaySessionProvider.notifier);
    try {
      final events = await session.queryRelay([
        NostrFilter(kinds: const [EventKind.presenceUpdate], authors: pubkeys),
      ]);
      var updated = state;
      var changed = false;
      for (final event in events) {
        final subject = _presenceSubject(event);
        if (!_tracked.contains(subject)) continue;
        final status = event.content;
        if (status != 'online' && status != 'away' && status != 'offline') {
          continue;
        }
        if (updated[subject] == status) continue;
        if (!changed) updated = Map<String, String>.from(updated);
        updated[subject] = status;
        changed = true;
      }
      if (changed) state = updated;
    } catch (error) {
      debugPrint('[PresenceCacheNotifier] presence fetch failed: $error');
    }
  }

  /// Subject pubkey of a presence event: the `p` tag for relay-synthesized
  /// events, the author for live self-published ones.
  String _presenceSubject(NostrEvent event) {
    for (final tag in event.tags) {
      if (tag.length >= 2 && tag[0] == 'p') return tag[1].toLowerCase();
    }
    return event.pubkey.toLowerCase();
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
