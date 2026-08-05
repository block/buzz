import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/profile/presence_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

/// Tests for [PresenceCacheNotifier]'s live updates and snapshot backstop.
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
    relaySession.emit(_presence('alice', 'away', createdAt: 1001));
    expect(container.read(presenceCacheProvider)['alice'], 'away');
  });

  test('track batches snapshot queries and maps relay p tags', () async {
    final relaySession = _RecordingRelaySessionNotifier()
      ..queueQueryResponse([
        _presence(
          'relay-pubkey',
          'online',
          createdAt: 1001,
          tags: const [
            ['p', 'Alice'],
          ],
        ),
        _presence('bob', 'away', createdAt: 1002),
      ]);
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    container.read(presenceCacheProvider.notifier)
      ..track(['ALICE'])
      ..track(['alice', 'BOB']);
    await _pumpSnapshotBatch();

    expect(relaySession.queryFilters, hasLength(1));
    final filter = relaySession.queryFilters.single.single;
    expect(filter.kinds, [EventKind.presenceUpdate]);
    expect(filter.authors, ['alice', 'bob']);
    expect(container.read(presenceCacheProvider), {
      'alice': 'online',
      'bob': 'away',
    });
  });

  test('snapshot keeps the latest event for each subject', () async {
    final relaySession = _RecordingRelaySessionNotifier()
      ..queueQueryResponse([
        _presence(
          'relay-pubkey',
          'online',
          createdAt: 1002,
          tags: const [
            ['p', 'alice'],
          ],
        ),
        _presence(
          'relay-pubkey',
          'away',
          createdAt: 1001,
          tags: const [
            ['p', 'alice'],
          ],
        ),
      ]);
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpSnapshotBatch();

    expect(container.read(presenceCacheProvider)['alice'], 'online');
  });

  test(
    'clock-skewed live event updates after a newer snapshot stamp',
    () async {
      final relaySession = _RecordingRelaySessionNotifier()
        ..queueQueryResponse([
          _presence(
            'relay-pubkey',
            'online',
            createdAt: 2000,
            tags: const [
              ['p', 'alice'],
            ],
          ),
        ]);
      final container = _buildContainer(relaySession: relaySession);
      addTearDown(container.dispose);

      container.read(presenceCacheProvider);
      container.read(presenceCacheProvider.notifier).track(['alice']);
      await _pumpSnapshotBatch();
      expect(container.read(presenceCacheProvider)['alice'], 'online');

      relaySession.emit(_presence('alice', 'away', createdAt: 1000));

      expect(container.read(presenceCacheProvider)['alice'], 'away');
    },
  );

  test(
    'snapshot marks requested pubkeys absent from the response offline',
    () async {
      final relaySession = _RecordingRelaySessionNotifier()
        ..queueQueryResponse([]);
      final container = _buildContainer(relaySession: relaySession);
      addTearDown(container.dispose);

      container.read(presenceCacheProvider);
      container.read(presenceCacheProvider.notifier).track(['alice']);
      relaySession.emit(_presence('alice', 'online'));
      await _pumpSnapshotBatch();

      expect(container.read(presenceCacheProvider)['alice'], 'offline');
    },
  );

  test(
    'snapshot absence does not overwrite a newer in-flight live event',
    () async {
      final response = Completer<List<NostrEvent>>();
      final relaySession = _RecordingRelaySessionNotifier()
        ..queueQueryFuture(response.future);
      final container = _buildContainer(relaySession: relaySession);
      addTearDown(container.dispose);

      container.read(presenceCacheProvider);
      container.read(presenceCacheProvider.notifier).track(['alice']);
      relaySession.emit(_presence('alice', 'away', createdAt: 2000));
      await _pumpSnapshotBatch();
      expect(relaySession.queryFilters, hasLength(1));

      relaySession.emit(_presence('alice', 'online', createdAt: 1000));
      response.complete([]);
      await _pumpEventQueue();

      expect(container.read(presenceCacheProvider)['alice'], 'online');
    },
  );

  test('reconnect refreshes all tracked pubkeys', () async {
    final relaySession = _RecordingRelaySessionNotifier()
      ..queueQueryResponse([_presence('alice', 'online')]);
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpSnapshotBatch();
    expect(container.read(presenceCacheProvider)['alice'], 'online');

    relaySession.setStatus(SessionStatus.disconnected);
    container.read(presenceCacheProvider);
    await _pumpEventQueue();
    relaySession.queueQueryResponse([]);
    relaySession.setStatus(SessionStatus.connected);
    container.read(presenceCacheProvider);
    await _pumpSnapshotBatch();

    expect(relaySession.queryFilters, hasLength(2));
    expect(container.read(presenceCacheProvider)['alice'], 'offline');
  });

  test('periodic refresh reconciles presence on a stable connection', () async {
    final relaySession = _RecordingRelaySessionNotifier()
      ..queueQueryResponse([_presence('alice', 'online')])
      ..queueQueryResponse([]);
    final container = _buildContainer(
      relaySession: relaySession,
      refreshInterval: const Duration(milliseconds: 100),
    );
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpSnapshotBatch();
    expect(container.read(presenceCacheProvider)['alice'], 'online');

    await _pumpPeriodicRefresh();

    expect(relaySession.queryFilters, hasLength(2));
    expect(container.read(presenceCacheProvider)['alice'], 'offline');
  });

  test(
    'periodic refresh stops while disconnected and after disposal',
    () async {
      final relaySession = _RecordingRelaySessionNotifier()
        ..queueQueryResponse([]);
      final container = _buildContainer(
        relaySession: relaySession,
        refreshInterval: const Duration(milliseconds: 100),
      );

      container.read(presenceCacheProvider);
      container.read(presenceCacheProvider.notifier).track(['alice']);
      await _pumpSnapshotBatch();
      expect(relaySession.queryFilters, hasLength(1));

      relaySession.setStatus(SessionStatus.disconnected);
      container.read(presenceCacheProvider);
      await _pumpPeriodicRefresh();
      expect(relaySession.queryFilters, hasLength(1));

      relaySession.queueQueryResponse([]);
      relaySession.setStatus(SessionStatus.connected);
      container.read(presenceCacheProvider);
      await _pumpSnapshotBatch();
      expect(relaySession.queryFilters, hasLength(2));

      container.dispose();
      relaySession.queueQueryResponse([]);
      await _pumpPeriodicRefresh();
      expect(relaySession.queryFilters, hasLength(2));
    },
  );

  test('relay snapshot reconciles a subject-clock-ahead live event', () async {
    final relaySession = _RecordingRelaySessionNotifier()
      ..queueQueryResponse([_presence('alice', 'online', createdAt: 1000)])
      ..queueQueryResponse([_presence('alice', 'online', createdAt: 2000)]);
    final container = _buildContainer(
      relaySession: relaySession,
      refreshInterval: const Duration(milliseconds: 100),
    );
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpSnapshotBatch();
    relaySession.emit(_presence('alice', 'away', createdAt: 3000));
    expect(container.read(presenceCacheProvider)['alice'], 'away');

    await _pumpPeriodicRefresh();

    expect(relaySession.queryFilters, hasLength(2));
    expect(container.read(presenceCacheProvider)['alice'], 'online');
  });

  test('failed snapshot is negatively cached against rebuild churn', () async {
    final relaySession = _RecordingRelaySessionNotifier()
      ..queueQueryError(StateError('snapshot failed'));
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    final notifier = container.read(presenceCacheProvider.notifier);
    notifier.track(['alice']);
    await _pumpSnapshotBatch();

    notifier.track(['alice']);
    await _pumpSnapshotBatch();

    expect(relaySession.queryFilters, hasLength(1));
  });

  test('snapshot publishes one state update for multiple subjects', () async {
    final relaySession = _RecordingRelaySessionNotifier()
      ..queueQueryResponse([
        _presence('alice', 'online'),
        _presence('bob', 'away'),
      ]);
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    var stateChangeCount = 0;
    container.listen(presenceCacheProvider, (prev, next) => stateChangeCount++);

    container.read(presenceCacheProvider.notifier).track(['alice', 'bob']);
    await _pumpSnapshotBatch();

    expect(stateChangeCount, 1);
    expect(container.read(presenceCacheProvider), {
      'alice': 'online',
      'bob': 'away',
    });
  });

  test(
    'stale pre-reconnect response cannot disturb the new snapshot',
    () async {
      final staleResponse = Completer<List<NostrEvent>>();
      final reconnectResponse = Completer<List<NostrEvent>>();
      final relaySession = _RecordingRelaySessionNotifier()
        ..queueQueryFuture(staleResponse.future)
        ..queueQueryFuture(reconnectResponse.future);
      final container = _buildContainer(relaySession: relaySession);
      addTearDown(container.dispose);

      container.read(presenceCacheProvider);
      final notifier = container.read(presenceCacheProvider.notifier);
      notifier.track(['alice']);
      await _pumpSnapshotBatch();

      relaySession.setStatus(SessionStatus.disconnected);
      container.read(presenceCacheProvider);
      await _pumpEventQueue();
      relaySession.setStatus(SessionStatus.connected);
      container.read(presenceCacheProvider);
      await _pumpSnapshotBatch();
      expect(relaySession.queryFilters, hasLength(2));

      staleResponse.complete([_presence('alice', 'away')]);
      await _pumpEventQueue();
      notifier.track(['alice']);
      await _pumpSnapshotBatch();
      expect(relaySession.queryFilters, hasLength(2));

      reconnectResponse.complete([_presence('alice', 'online')]);
      await _pumpEventQueue();
      expect(container.read(presenceCacheProvider)['alice'], 'online');
    },
  );

  test('track deduplicates in-flight and recently fetched snapshots', () async {
    final response = Completer<List<NostrEvent>>();
    final relaySession = _RecordingRelaySessionNotifier()
      ..queueQueryFuture(response.future);
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    final notifier = container.read(presenceCacheProvider.notifier);
    notifier.track(['alice']);
    await _pumpSnapshotBatch();
    notifier.track(['alice']);
    await _pumpSnapshotBatch();
    expect(relaySession.queryFilters, hasLength(1));

    response.complete([_presence('alice', 'online')]);
    await _pumpEventQueue();
    notifier.track(['alice']);
    await _pumpSnapshotBatch();

    expect(relaySession.queryFilters, hasLength(1));
  });

  test('disposing ignores an in-flight snapshot response', () async {
    final response = Completer<List<NostrEvent>>();
    final relaySession = _RecordingRelaySessionNotifier()
      ..queueQueryFuture(response.future);
    final container = _buildContainer(relaySession: relaySession);

    container.read(presenceCacheProvider);
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpSnapshotBatch();
    expect(relaySession.queryFilters, hasLength(1));

    container.dispose();
    response.complete([_presence('alice', 'online')]);
    await _pumpEventQueue();
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

NostrEvent _presence(
  String pubkey,
  String status, {
  int createdAt = 1000,
  List<List<String>> tags = const [],
}) => NostrEvent(
  id: 'evt-$pubkey-$status',
  pubkey: pubkey,
  createdAt: createdAt,
  kind: EventKind.presenceUpdate,
  tags: tags,
  content: status,
  sig: 'sig',
);

Future<void> _pumpEventQueue() async {
  await Future<void>.delayed(Duration.zero);
  await Future<void>.delayed(Duration.zero);
}

Future<void> _pumpSnapshotBatch() async {
  await Future<void>.delayed(const Duration(milliseconds: 75));
  await _pumpEventQueue();
}

Future<void> _pumpPeriodicRefresh() async {
  await Future<void>.delayed(const Duration(milliseconds: 110));
  await _pumpEventQueue();
}

ProviderContainer _buildContainer({
  required _RecordingRelaySessionNotifier relaySession,
  Duration? refreshInterval,
}) {
  return ProviderContainer(
    overrides: [
      appLifecycleProvider.overrideWith(() => _FakeAppLifecycleNotifier()),
      relaySessionProvider.overrideWith(() => relaySession),
      if (refreshInterval != null)
        presenceCacheProvider.overrideWith(
          () => PresenceCacheNotifier(refreshInterval: refreshInterval),
        ),
    ],
  );
}

class _RecordingRelaySessionNotifier extends RelaySessionNotifier {
  final List<NostrFilter> filters = [];
  final List<List<NostrFilter>> queryFilters = [];
  final List<Future<List<NostrEvent>>> _queryResponses = [];
  final List<Object> _queryErrors = [];
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

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    queryFilters.add(filters);
    if (_queryErrors.isNotEmpty) throw _queryErrors.removeAt(0);
    if (_queryResponses.isEmpty) return [];
    return _queryResponses.removeAt(0);
  }

  void queueQueryResponse(List<NostrEvent> events) {
    _queryResponses.add(Future.value(events));
  }

  void queueQueryFuture(Future<List<NostrEvent>> events) {
    _queryResponses.add(events);
  }

  void queueQueryError(Object error) {
    _queryErrors.add(error);
  }

  void setStatus(SessionStatus status) {
    state = SessionState(status: status);
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
