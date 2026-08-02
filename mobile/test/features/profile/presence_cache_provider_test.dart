import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/profile/presence_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

/// Tests for [PresenceCacheNotifier]'s subscribe-first live + snapshot flow.
void main() {
  test('current online snapshot seeds a newly tracked pubkey', () async {
    final relaySession = _RecordingRelaySessionNotifier(
      queryResult: [_presenceSnapshot('relay-signer', 'alice', 'online')],
    );
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();

    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpEventQueue();

    expect(container.read(presenceCacheProvider)['alice'], 'online');
    expect(relaySession.operations, ['subscribe', 'query']);
  });

  test('waits for the live subscription before querying snapshots', () async {
    final subscriptionReady = Completer<void>();
    final relaySession = _RecordingRelaySessionNotifier(
      subscribeGate: subscriptionReady,
    );
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpEventQueue();

    expect(relaySession.operations, ['subscribe']);
    expect(relaySession.queries, isEmpty);

    subscriptionReady.complete();
    await _pumpEventQueue();

    expect(relaySession.operations, ['subscribe', 'query']);
  });

  test('relay-signed snapshot uses p-tag subject, not event author', () async {
    final relaySession = _RecordingRelaySessionNotifier(
      queryResult: [_presenceSnapshot('relay-signer', 'alice', 'away')],
    );
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpEventQueue();

    final cache = container.read(presenceCacheProvider);
    expect(cache['alice'], 'away');
    expect(cache.containsKey('relay-signer'), isFalse);
  });

  test('live update wins when snapshot query completes later', () async {
    final queryCompleter = Completer<List<NostrEvent>>();
    final relaySession = _RecordingRelaySessionNotifier(
      queryHandler: (_) => queryCompleter.future,
    );
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpEventQueue();

    relaySession.emit(_presence('alice', 'away'));
    queryCompleter.complete([
      _presenceSnapshot('relay-signer', 'alice', 'online'),
    ]);
    await _pumpEventQueue();

    expect(container.read(presenceCacheProvider)['alice'], 'away');
  });

  test('tracking the same pubkey does not repeat the snapshot query', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();

    container.read(presenceCacheProvider.notifier).track(['alice']);
    container.read(presenceCacheProvider.notifier).track(['ALICE', 'alice']);
    await _pumpEventQueue();
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpEventQueue();

    expect(relaySession.queries, hasLength(1));
    expect(relaySession.queries.single.single.authors, ['alice']);
  });

  test('successful empty snapshot resolves tracked pubkey offline', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpEventQueue();

    expect(container.read(presenceCacheProvider)['alice'], 'offline');
  });

  test('failed snapshot query retries automatically', () async {
    var attempts = 0;
    final relaySession = _RecordingRelaySessionNotifier(
      queryHandler: (_) {
        attempts++;
        if (attempts == 1) {
          return Future.error(Exception('temporary query failure'));
        }
        return Future.value([
          _presenceSnapshot('relay-signer', 'alice', 'online'),
        ]);
      },
    );
    final container = _buildContainer(
      relaySession: relaySession,
      snapshotRetryBaseDelay: Duration.zero,
    );
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpEventQueue();

    expect(attempts, 2);
    expect(container.read(presenceCacheProvider)['alice'], 'online');
  });

  test(
    'initial subscription failure retries while session stays connected',
    () async {
      final relaySession = _RecordingRelaySessionNotifier(
        subscribeFailuresRemaining: 1,
      );
      final container = _buildContainer(
        relaySession: relaySession,
        subscriptionRetryBaseDelay: Duration.zero,
      );
      addTearDown(container.dispose);

      container.read(presenceCacheProvider);
      container.read(presenceCacheProvider.notifier).track(['alice']);
      await _pumpEventQueue();

      expect(
        relaySession.operations.where((operation) => operation == 'subscribe'),
        hasLength(2),
      );
      expect(relaySession.queries, hasLength(1));
      expect(container.read(presenceCacheProvider)['alice'], 'offline');
    },
  );

  test(
    'late CLOSED resubscribes, resnapshots, and resumes live updates',
    () async {
      final relaySession = _RecordingRelaySessionNotifier(
        queryResult: [_presenceSnapshot('relay-signer', 'alice', 'online')],
      );
      final container = _buildContainer(
        relaySession: relaySession,
        subscriptionRetryBaseDelay: Duration.zero,
      );
      addTearDown(container.dispose);

      container.read(presenceCacheProvider);
      await _pumpEventQueue();
      container.read(presenceCacheProvider.notifier).track(['alice']);
      await _pumpEventQueue();
      expect(container.read(presenceCacheProvider)['alice'], 'online');

      relaySession.closeLatest('relay maintenance');
      expect(
        container.read(presenceCacheProvider).containsKey('alice'),
        isFalse,
      );
      await _pumpEventQueue();

      expect(
        relaySession.operations.where((operation) => operation == 'subscribe'),
        hasLength(2),
      );
      expect(relaySession.queries, hasLength(2));

      relaySession.emit(_presence('alice', 'away'));
      expect(container.read(presenceCacheProvider)['alice'], 'away');
    },
  );

  test(
    'dispose invalidates a delayed subscription without leaking it',
    () async {
      final subscriptionReady = Completer<void>();
      final relaySession = _RecordingRelaySessionNotifier(
        subscribeGate: subscriptionReady,
      );
      final container = _buildContainer(relaySession: relaySession);

      container.read(presenceCacheProvider);
      container.read(presenceCacheProvider.notifier).track(['alice']);
      await _pumpEventQueue();
      container.dispose();

      subscriptionReady.complete();
      await _pumpEventQueue();

      expect(relaySession.unsubscribeCount, 1);
      expect(relaySession.activeSubscriptionCount, 0);
      expect(relaySession.queries, isEmpty);
    },
  );

  test('dispose invalidates an in-flight snapshot query', () async {
    final queryCompleter = Completer<List<NostrEvent>>();
    final relaySession = _RecordingRelaySessionNotifier(
      queryHandler: (_) => queryCompleter.future,
    );
    final container = _buildContainer(relaySession: relaySession);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpEventQueue();
    expect(relaySession.queries, hasLength(1));

    container.dispose();
    queryCompleter.complete([
      _presenceSnapshot('relay-signer', 'alice', 'online'),
    ]);
    await _pumpEventQueue();

    expect(relaySession.activeSubscriptionCount, 0);
  });

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

NostrEvent _presenceSnapshot(
  String relayPubkey,
  String subjectPubkey,
  String status,
) => NostrEvent(
  id: 'snapshot-$subjectPubkey-$status',
  pubkey: relayPubkey,
  createdAt: 1000,
  kind: EventKind.presenceUpdate,
  tags: [
    ['p', subjectPubkey],
  ],
  content: status,
  sig: 'relay-sig',
);

Future<void> _pumpEventQueue() async {
  for (var i = 0; i < 5; i++) {
    await Future<void>.delayed(Duration.zero);
  }
}

ProviderContainer _buildContainer({
  required _RecordingRelaySessionNotifier relaySession,
  Duration subscriptionRetryBaseDelay = const Duration(seconds: 1),
  Duration snapshotRetryBaseDelay = const Duration(seconds: 1),
}) {
  return ProviderContainer(
    overrides: [
      appLifecycleProvider.overrideWith(() => _FakeAppLifecycleNotifier()),
      relaySessionProvider.overrideWith(() => relaySession),
      presenceCacheProvider.overrideWith(
        () => PresenceCacheNotifier(
          subscriptionRetryBaseDelay: subscriptionRetryBaseDelay,
          snapshotRetryBaseDelay: snapshotRetryBaseDelay,
        ),
      ),
    ],
  );
}

class _RecordingRelaySessionNotifier extends RelaySessionNotifier {
  _RecordingRelaySessionNotifier({
    List<NostrEvent> queryResult = const [],
    Future<List<NostrEvent>> Function(List<NostrFilter>)? queryHandler,
    Completer<void>? subscribeGate,
    int subscribeFailuresRemaining = 0,
  }) : _queryResult = queryResult,
       _queryHandler = queryHandler,
       _subscribeGate = subscribeGate,
       _subscribeFailuresRemaining = subscribeFailuresRemaining;

  final List<NostrEvent> _queryResult;
  final Future<List<NostrEvent>> Function(List<NostrFilter>)? _queryHandler;
  final Completer<void>? _subscribeGate;
  int _subscribeFailuresRemaining;
  final List<_RecordedSubscription> _subscriptions = [];
  final List<List<NostrFilter>> queries = [];
  final List<String> operations = [];
  int unsubscribeCount = 0;

  List<NostrFilter> get filters => [
    for (final subscription in _subscriptions) subscription.filter,
  ];

  int get activeSubscriptionCount => _subscriptions.length;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    operations.add('subscribe');
    await _subscribeGate?.future;
    if (_subscribeFailuresRemaining > 0) {
      _subscribeFailuresRemaining--;
      throw Exception('temporary subscription failure');
    }
    final subscription = _RecordedSubscription(
      filter: filter,
      onEvent: onEvent,
      onClosed: onClosed,
    );
    _subscriptions.add(subscription);
    return () {
      if (!_subscriptions.remove(subscription)) return;
      unsubscribeCount++;
    };
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    operations.add('query');
    queries.add(filters);
    return _queryHandler?.call(filters) ?? Future.value(_queryResult);
  }

  /// Emit an event synchronously to all live subscribers.
  void emit(NostrEvent event) {
    for (final subscription in List.of(_subscriptions)) {
      subscription.onEvent(event);
    }
  }

  void closeLatest(String message) {
    final subscription = _subscriptions.removeLast();
    subscription.onClosed?.call(message);
  }
}

class _RecordedSubscription {
  final NostrFilter filter;
  final void Function(NostrEvent) onEvent;
  final void Function(String message)? onClosed;

  const _RecordedSubscription({
    required this.filter,
    required this.onEvent,
    required this.onClosed,
  });
}

class _FakeAppLifecycleNotifier extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}
