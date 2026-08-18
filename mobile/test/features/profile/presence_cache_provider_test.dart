import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/profile/presence_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

/// Tests for [PresenceCacheNotifier] in the pure-Nostr world.
///
/// The cache is now purely WS-driven: the notifier subscribes to kind:20001
/// (presence updates) over the relay session and only mutates state for
/// pubkeys that have been registered via [PresenceCacheNotifier.track].
/// There is no longer a REST backstop — the previous test seeded state via
/// a `GET /api/presence` call which has been removed.
void main() {
  test('WS presence event updates cache for tracked pubkey', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    // Initialize the notifier (triggers build → subscribes to WS).
    container.read(presenceCacheProvider);
    await _pumpEventQueue();

    // Track alice, then emit her initial 'online' status.
    container.read(presenceCacheProvider.notifier).track(['alice']);
    relaySession.emit(_presence('alice', 'online'));
    expect(container.read(presenceCacheProvider)['alice'], 'online');

    // Simulate a WS presence event: alice goes away.
    relaySession.emit(_presence('alice', 'away'));
    expect(container.read(presenceCacheProvider)['alice'], 'away');
  });

  test('WS presence event ignores untracked pubkeys', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();

    // Track only alice.
    container.read(presenceCacheProvider.notifier).track(['alice']);

    // Emit event for bob (untracked).
    relaySession.emit(_presence('bob', 'online'));

    // Bob should NOT appear in the cache.
    expect(container.read(presenceCacheProvider).containsKey('bob'), isFalse);
  });

  test('WS presence event ignores invalid status values', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();

    container.read(presenceCacheProvider.notifier).track(['alice']);
    relaySession.emit(_presence('alice', 'online'));
    expect(container.read(presenceCacheProvider)['alice'], 'online');

    // Emit event with garbage status — should be rejected.
    relaySession.emit(_presence('alice', 'garbage-status'));

    // Status should remain 'online'.
    expect(container.read(presenceCacheProvider)['alice'], 'online');
  });

  test('WS presence event skips no-op updates', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();

    container.read(presenceCacheProvider.notifier).track(['alice']);
    relaySession.emit(_presence('alice', 'online'));

    // Listen for state changes after initial setup.
    var stateChangeCount = 0;
    container.listen(presenceCacheProvider, (prev, next) => stateChangeCount++);

    // Emit event with same status as current.
    relaySession.emit(_presence('alice', 'online'));

    // No state change should occur — it's a no-op.
    expect(stateChangeCount, 0);
  });

  test('subscribes to kind:20001 with limit 0', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();

    // Should have subscribed with the correct filter.
    expect(relaySession.filters, hasLength(1));
    expect(relaySession.filters.single.kinds, [EventKind.presenceUpdate]);
    expect(relaySession.filters.single.limit, 0);
  });

  test('WS event uses pubkey variable, not literal string', () async {
    // Regression test for the map key bug where `{...state, pubkey: status}`
    // used the literal string "pubkey" instead of the variable's value.
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();

    container.read(presenceCacheProvider.notifier).track([
      'deadbeef',
      'cafebabe',
    ]);

    // Seed cafebabe -> offline, then set deadbeef online.
    relaySession.emit(_presence('cafebabe', 'offline'));
    relaySession.emit(_presence('deadbeef', 'online'));

    final cache = container.read(presenceCacheProvider);
    // deadbeef should be online (the actual pubkey, not a literal "pubkey" key).
    expect(cache['deadbeef'], 'online');
    // cafebabe should still be offline (not clobbered).
    expect(cache['cafebabe'], 'offline');
    // There should be no literal "pubkey" key in the map.
    expect(cache.containsKey('pubkey'), isFalse);
  });
}

NostrEvent _presence(String pubkey, String status) => NostrEvent(
  id: 'evt-$pubkey-$status',
  pubkey: pubkey,
  createdAt: 1000,
  kind: EventKind.presenceUpdate,
  tags: const [],
  content: status,
  sig: 'sig',
);

Future<void> _pumpEventQueue() async {
  await Future<void>.delayed(Duration.zero);
  await Future<void>.delayed(Duration.zero);
}

ProviderContainer _buildContainer({
  required _RecordingRelaySessionNotifier relaySession,
}) {
  return ProviderContainer(
    overrides: [
      appLifecycleProvider.overrideWith(() => _FakeAppLifecycleNotifier()),
      relaySessionProvider.overrideWith(() => relaySession),
    ],
  );
}

class _RecordingRelaySessionNotifier extends RelaySessionNotifier {
  final List<NostrFilter> filters = [];
  final List<void Function(NostrEvent)> _listeners = [];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    filters.add(filter);
    _listeners.add(onEvent);
    return () {
      filters.remove(filter);
      _listeners.remove(onEvent);
    };
  }

  /// Emit an event synchronously to all live subscribers.
  void emit(NostrEvent event) {
    for (final listener in List.of(_listeners)) {
      listener(event);
    }
  }
}

class _FakeAppLifecycleNotifier extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}
