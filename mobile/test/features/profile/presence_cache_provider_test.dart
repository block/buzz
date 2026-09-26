import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/profile/presence_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  test('WS presence event updates cache for tracked pubkey', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _waitFor(() => relaySession.filters.isNotEmpty);

    container.read(presenceCacheProvider.notifier).track(['alice']);
    relaySession.emit(_presence('alice', 'online'));
    expect(container.read(presenceCacheProvider)['alice'], 'online');

    relaySession.emit(_presence('alice', 'away', createdAt: 1001));
    expect(container.read(presenceCacheProvider)['alice'], 'away');
  });

  test(
    'backfills tracked presence from authenticated query snapshot',
    () async {
      final relaySession = _RecordingRelaySessionNotifier()
        ..queryResults = [_snapshot('alice', 'online')];
      final container = _buildContainer(relaySession: relaySession);
      addTearDown(container.dispose);

      container.read(presenceCacheProvider);
      await _waitFor(() => relaySession.filters.isNotEmpty);
      container.read(presenceCacheProvider.notifier).track(['ALICE']);

      await _waitFor(
        () => container.read(presenceCacheProvider)['alice'] == 'online',
      );
      expect(relaySession.queries, hasLength(1));
      final filter = relaySession.queries.single.single;
      expect(filter.kinds, [EventKind.presenceUpdate]);
      expect(filter.authors, ['alice']);
      expect(filter.limit, 1);
    },
  );

  test('snapshot marks a missing tracked identity offline', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _waitFor(() => relaySession.filters.isNotEmpty);
    container.read(presenceCacheProvider.notifier).track(['alice']);

    await _waitFor(
      () => container.read(presenceCacheProvider)['alice'] == 'offline',
    );
  });

  test('live heartbeat wins over an older in-flight snapshot', () async {
    final snapshotCompleter = Completer<List<NostrEvent>>();
    final relaySession = _RecordingRelaySessionNotifier()
      ..queryCompleter = snapshotCompleter;
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _waitFor(() => relaySession.filters.isNotEmpty);
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _waitFor(() => relaySession.queries.isNotEmpty);

    relaySession.emit(_presence('alice', 'online'));
    snapshotCompleter.complete([_snapshot('alice', 'away')]);
    await _pumpEventQueue();

    expect(container.read(presenceCacheProvider)['alice'], 'online');
  });

  test('fresh live status survives a temporarily empty snapshot', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _waitFor(() => relaySession.filters.isNotEmpty);
    container.read(presenceCacheProvider.notifier).track(['alice']);
    relaySession.emit(_presence('alice', 'online'));

    await _waitFor(() => relaySession.queries.isNotEmpty);
    await _pumpEventQueue();

    expect(container.read(presenceCacheProvider)['alice'], 'online');
  });

  test('live event never trusts a spoofed p-tag subject', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _waitFor(() => relaySession.filters.isNotEmpty);
    container.read(presenceCacheProvider.notifier).track(['alice']);
    relaySession.emit(
      _presence(
        'mallory',
        'online',
        tags: const [
          ['p', 'alice'],
        ],
      ),
    );

    expect(container.read(presenceCacheProvider).containsKey('alice'), isFalse);
  });

  test(
    'WS presence event ignores untracked pubkeys and invalid status',
    () async {
      final relaySession = _RecordingRelaySessionNotifier();
      final container = _buildContainer(relaySession: relaySession);
      addTearDown(container.dispose);

      container.read(presenceCacheProvider);
      await _waitFor(() => relaySession.filters.isNotEmpty);
      container.read(presenceCacheProvider.notifier).track(['alice']);

      relaySession.emit(_presence('bob', 'online'));
      relaySession.emit(_presence('alice', 'garbage-status'));

      final cache = container.read(presenceCacheProvider);
      expect(cache.containsKey('bob'), isFalse);
      expect(cache.containsKey('alice'), isFalse);
    },
  );

  test(
    'older replayed heartbeat cannot overwrite a newer live status',
    () async {
      final relaySession = _RecordingRelaySessionNotifier();
      final container = _buildContainer(relaySession: relaySession);
      addTearDown(container.dispose);

      container.read(presenceCacheProvider);
      await _waitFor(() => relaySession.filters.isNotEmpty);
      container.read(presenceCacheProvider.notifier).track(['alice']);

      relaySession.emit(_presence('alice', 'away', createdAt: 1001));
      relaySession.emit(_presence('alice', 'online', createdAt: 1000));

      expect(container.read(presenceCacheProvider)['alice'], 'away');
    },
  );

  test('snapshot timestamp rejects an older replayed live heartbeat', () async {
    final relaySession = _RecordingRelaySessionNotifier()
      ..queryResults = [_snapshot('alice', 'online')];
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _waitFor(() => relaySession.filters.isNotEmpty);
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _waitFor(
      () => container.read(presenceCacheProvider)['alice'] == 'online',
    );

    relaySession.emit(_presence('alice', 'away', createdAt: 1999));

    expect(container.read(presenceCacheProvider)['alice'], 'online');
  });

  test(
    'keeps cached presence during reconnect and then resynchronizes',
    () async {
      final relaySession = _RecordingRelaySessionNotifier()
        ..queryResults = [_snapshot('alice', 'online')];
      final container = _buildContainer(relaySession: relaySession);
      addTearDown(container.dispose);

      container.read(presenceCacheProvider);
      await _waitFor(() => relaySession.filters.isNotEmpty);
      container.read(presenceCacheProvider.notifier).track(['alice']);
      await _waitFor(
        () => container.read(presenceCacheProvider)['alice'] == 'online',
      );

      relaySession.setStatus(SessionStatus.reconnecting);
      container.read(presenceCacheProvider);
      await _pumpEventQueue();
      expect(container.read(presenceCacheProvider)['alice'], 'online');

      relaySession.queryResults = [_snapshot('alice', 'away')];
      final queryCount = relaySession.queries.length;
      relaySession.setStatus(SessionStatus.connected);
      container.read(presenceCacheProvider);
      await _waitFor(() => relaySession.queries.length > queryCount);
      await _waitFor(
        () => container.read(presenceCacheProvider)['alice'] == 'away',
      );
    },
  );

  test('subscribes to kind:20001 with limit 0', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _waitFor(() => relaySession.filters.isNotEmpty);

    expect(relaySession.filters, hasLength(1));
    expect(relaySession.filters.single.kinds, [EventKind.presenceUpdate]);
    expect(relaySession.filters.single.limit, 0);
  });

  test(
    'relay change clears cache and discards an in-flight snapshot',
    () async {
      final relayConfig = _FakeRelayConfigNotifier();
      final relaySession = _RecordingRelaySessionNotifier()
        ..queryResults = [_snapshot('alice', 'online')];
      final container = _buildContainer(
        relaySession: relaySession,
        relayConfig: relayConfig,
      );
      addTearDown(container.dispose);

      container.read(presenceCacheProvider);
      await _waitFor(() => relaySession.filters.isNotEmpty);
      container.read(presenceCacheProvider.notifier).track(['alice']);
      await _waitFor(
        () => container.read(presenceCacheProvider)['alice'] == 'online',
      );

      final staleQuery = Completer<List<NostrEvent>>();
      relaySession.queryCompleter = staleQuery;
      final queryCount = relaySession.queries.length;
      container.read(presenceCacheProvider.notifier).track(['bob']);
      await _waitFor(() => relaySession.queries.length > queryCount);

      relayConfig.setRelay('https://other-relay.example');
      container.read(presenceCacheProvider);
      expect(container.read(presenceCacheProvider), isEmpty);

      staleQuery.complete([_snapshot('bob', 'online')]);
      await _pumpEventQueue();

      expect(container.read(presenceCacheProvider), isEmpty);
    },
  );

  test('dispose discards an in-flight snapshot', () async {
    final snapshotCompleter = Completer<List<NostrEvent>>();
    final relaySession = _RecordingRelaySessionNotifier()
      ..queryCompleter = snapshotCompleter;
    final container = _buildContainer(relaySession: relaySession);

    container.read(presenceCacheProvider);
    await _waitFor(() => relaySession.filters.isNotEmpty);
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _waitFor(() => relaySession.queries.isNotEmpty);

    container.dispose();
    snapshotCompleter.complete([_snapshot('alice', 'online')]);
    await _pumpEventQueue();

    expect(relaySession.filters, isEmpty);
  });
}

NostrEvent _presence(
  String pubkey,
  String status, {
  int createdAt = 1000,
  List<List<String>> tags = const [],
}) => NostrEvent(
  id: 'evt-$pubkey-$status-$createdAt',
  pubkey: pubkey,
  createdAt: createdAt,
  kind: EventKind.presenceUpdate,
  tags: tags,
  content: status,
  sig: 'sig',
);

NostrEvent _snapshot(String subject, String status) => NostrEvent(
  id: 'snapshot-$subject-$status',
  pubkey: 'relay',
  createdAt: 2000,
  kind: EventKind.presenceUpdate,
  tags: [
    ['p', subject],
  ],
  content: status,
  sig: 'relay-sig',
);

Future<void> _pumpEventQueue() async {
  await Future<void>.delayed(Duration.zero);
  await Future<void>.delayed(Duration.zero);
}

Future<void> _waitFor(bool Function() predicate) async {
  for (var attempt = 0; attempt < 50; attempt++) {
    if (predicate()) return;
    await Future<void>.delayed(const Duration(milliseconds: 10));
  }
  fail('condition was not met before timeout');
}

ProviderContainer _buildContainer({
  required _RecordingRelaySessionNotifier relaySession,
  _FakeRelayConfigNotifier? relayConfig,
}) {
  return ProviderContainer(
    overrides: [
      appLifecycleProvider.overrideWith(() => _FakeAppLifecycleNotifier()),
      relayConfigProvider.overrideWith(
        () => relayConfig ?? _FakeRelayConfigNotifier(),
      ),
      relaySessionProvider.overrideWith(() => relaySession),
    ],
  );
}

class _RecordingRelaySessionNotifier extends RelaySessionNotifier {
  final List<NostrFilter> filters = [];
  final List<List<NostrFilter>> queries = [];
  final List<void Function(NostrEvent)> _listeners = [];
  List<NostrEvent> queryResults = [];
  Completer<List<NostrEvent>>? queryCompleter;

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
  }) {
    queries.add(filters);
    return queryCompleter?.future ?? Future.value(queryResults);
  }

  void emit(NostrEvent event) {
    for (final listener in List.of(_listeners)) {
      listener(event);
    }
  }

  void setStatus(SessionStatus status) {
    state = SessionState(status: status);
  }
}

class _FakeAppLifecycleNotifier extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(baseUrl: 'https://relay.example');

  void setRelay(String baseUrl) {
    state = RelayConfig(baseUrl: baseUrl);
  }
}
