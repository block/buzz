import 'dart:async';

import 'package:buzz/features/profile/presence_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  void scenario(
    void Function(FakeAsync, ProviderContainer, PresenceTestRelay) run,
  ) {
    fakeAsync((time) {
      final relay = PresenceTestRelay();
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => relay),
          appLifecycleProvider.overrideWith(_Lifecycle.new),
        ],
      );
      container.read(presenceCacheProvider.notifier).track([' ALICE ', 'bob']);
      time.elapse(Duration.zero);
      try {
        run(time, container, relay);
      } finally {
        container.dispose();
        time.flushMicrotasks();
      }
    });
  }

  test('snapshot covers exact subjects; latest p-tag record wins', () {
    scenario((time, container, relay) {
      expect(relay.queries.single.authors, ['alice', 'bob']);
      expect(relay.queries.single.limit, 2);
      expect(relay.queries.single.kinds, [EventKind.presenceUpdate]);
      relay.results.removeAt(0).complete([
        presenceEvent('relay', 'away', subject: 'alice', timestamp: 20),
        presenceEvent('relay', ' online ', subject: 'alice'),
        presenceEvent('stranger', 'online'),
      ]);
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider), {
        'alice': 'away',
        'bob': 'offline',
      });
      container.read(presenceCacheProvider.notifier).track(['alice']);
      time.elapse(Duration.zero);
      expect(relay.queries, hasLength(1));
    });
  });

  test('unchanged live heartbeat fences snapshot and older live events', () {
    scenario((time, container, relay) {
      relay.emit(presenceEvent('alice', 'online'));
      relay.results.removeAt(0).complete([]);
      time.flushMicrotasks();
      time.elapse(const Duration(seconds: 60));
      relay.emit(presenceEvent('alice', 'online', timestamp: 20));
      relay.emit(presenceEvent('alice', 'offline', timestamp: 1));
      relay.results.removeAt(0).complete([]);
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider)['alice'], 'online');
      // No new heartbeat: a successful empty refresh observes relay TTL expiry.
      time.elapse(const Duration(seconds: 60));
      relay.results.removeAt(0).complete([]);
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider)['alice'], 'offline');
    });
  });

  test('failed initial and refresh reads are unknown and retry on poll', () {
    scenario((time, container, relay) {
      relay.results.removeAt(0).completeError(Exception('unavailable'));
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider), isEmpty);
      time.elapse(const Duration(seconds: 60));
      relay.results.removeAt(0).complete([presenceEvent('alice', 'online')]);
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider)['alice'], 'online');
      time.elapse(const Duration(seconds: 60));
      relay.results.removeAt(0).completeError(Exception('unavailable'));
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider), isEmpty);
    });
  });

  for (final timestamp in [10, 20]) {
    for (final settled in ['online', 'offline']) {
      final transition = settled == 'online' ? 'offline' : 'online';
      test('ambiguous $timestamp/$settled fences and freshly confirms', () {
        scenario((time, container, relay) {
          final online = presenceEvent(
            'relay',
            'online',
            subject: 'alice',
            timestamp: 20,
          );
          relay.results.removeAt(0).complete([online]);
          time.flushMicrotasks();
          if (settled == 'offline') {
            time.elapse(const Duration(seconds: 60));
            relay.results.removeAt(0).complete([]);
            time.flushMicrotasks();
          }
          final snapshot = settled == 'online' ? [online] : <NostrEvent>[];
          expect(container.read(presenceCacheProvider)['alice'], settled);
          time.elapse(const Duration(seconds: 60));
          final stale = relay.results.removeAt(0);
          final delayed = presenceEvent(
            'alice',
            transition,
            timestamp: timestamp,
          );
          relay.emit(delayed);
          expect(container.read(presenceCacheProvider)['alice'], settled);
          stale.complete([]);
          time.flushMicrotasks();
          expect(container.read(presenceCacheProvider)['alice'], settled);
          relay.results.removeAt(0).complete(snapshot);
          time.flushMicrotasks();
          expect(container.read(presenceCacheProvider)['alice'], settled);
          relay.emit(delayed);
          time.elapse(Duration.zero);
          expect(relay.queries, hasLength(settled == 'online' ? 3 : 4));
          relay.emit(
            presenceEvent(
              'alice',
              transition,
              timestamp: timestamp,
              subject: 'new',
            ),
          );
          time.elapse(Duration.zero);
          relay.results
              .removeAt(0)
              .complete(transition == 'online' ? [online] : []);
          time.flushMicrotasks();
          expect(container.read(presenceCacheProvider)['alice'], transition);
          relay.emit(presenceEvent('alice', settled, timestamp: 21));
          expect(container.read(presenceCacheProvider)['alice'], settled);
        });
      });
    }
  }

  test('post-ready retry invalidates in-flight results and resnapshots', () {
    scenario((time, container, relay) {
      final stale = relay.results.removeAt(0);
      relay.status(RelaySubscriptionStatus.retrying);
      stale.complete([presenceEvent('alice', 'online')]);
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider), isEmpty);
      relay.status(RelaySubscriptionStatus.ready);
      time.elapse(Duration.zero);
      expect(relay.queries, hasLength(2));
      relay.results.removeAt(0).complete([presenceEvent('alice', 'away')]);
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider)['alice'], 'away');
    });
  });

  test('terminal close fences late events and recovers at bounded cadence', () {
    scenario((time, container, relay) {
      final emitOld = relay.emit;
      final stale = relay.results.removeAt(0);
      relay.close('restricted');
      emitOld(presenceEvent('alice', 'online'));
      stale.complete([presenceEvent('alice', 'online')]);
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider), isEmpty);
      time.elapse(const Duration(seconds: 59));
      expect(relay.subscriptions, 1);
      time.elapse(const Duration(seconds: 1));
      expect(relay.subscriptions, 2);
      relay.results.removeAt(0).complete([]);
      time.flushMicrotasks();
      relay.emit(presenceEvent('alice', 'away'));
      expect(container.read(presenceCacheProvider)['alice'], 'away');
    });
  });

  test('disconnect and disposal fence pending queries and unsubscribe', () {
    scenario((time, container, relay) {
      final stale = relay.results.removeAt(0);
      relay.state = const SessionState(status: SessionStatus.disconnected);
      expect(container.read(presenceCacheProvider), isEmpty);
      stale.complete([presenceEvent('alice', 'online')]);
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider), isEmpty);
      relay.state = const SessionState(status: SessionStatus.connected);
      container.read(presenceCacheProvider);
      time.elapse(Duration.zero);
      final disposed = relay.results.removeAt(0);
      container.dispose();
      disposed.complete([presenceEvent('alice', 'online')]);
      time.flushMicrotasks();
      expect(relay.unsubscribes, 2);
    });
  });

  test('community/account switch clears tracking and fences old snapshot', () {
    scenario((time, container, relay) {
      final stale = relay.results.removeAt(0);
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://other.example', nsec: 'different-account');
      expect(container.read(presenceCacheProvider), isEmpty);
      time.elapse(Duration.zero);
      stale.complete([presenceEvent('alice', 'online')]);
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider), isEmpty);
      expect(relay.queries, hasLength(1));
      container.read(presenceCacheProvider.notifier).track(['carol']);
      time.elapse(Duration.zero);
      expect(relay.queries.last.authors, ['carol']);
    });
  });

  test('background stops polling; foreground obtains a fresh snapshot', () {
    scenario((time, container, relay) {
      relay.results.removeAt(0).complete([presenceEvent('alice', 'online')]);
      time.flushMicrotasks();
      container.read(appLifecycleProvider.notifier).state =
          AppLifecycleState.paused;
      expect(container.read(presenceCacheProvider), isEmpty);
      time.elapse(const Duration(minutes: 2));
      expect(relay.queries, hasLength(1));
      container.read(appLifecycleProvider.notifier).state =
          AppLifecycleState.resumed;
      container.read(presenceCacheProvider);
      time.elapse(Duration.zero);
      expect(relay.queries, hasLength(2));
    });
  });

  test(
    'batches beyond default limit without declaring unqueried keys offline',
    () {
      scenario((time, container, relay) {
        container.read(presenceCacheProvider.notifier).track([
          for (var i = 0; i < 205; i++) 'agent-$i',
        ]);
        relay.results.removeAt(0).complete([]);
        time.flushMicrotasks();
        expect(relay.queries.last.authors, hasLength(100));
        expect(container.read(presenceCacheProvider)['agent-204'], isNull);
        for (var i = 0; i < 3; i++) {
          relay.results.removeAt(0).complete([]);
          time.flushMicrotasks();
        }
        expect(relay.queries.map((filter) => filter.limit), [2, 100, 100, 5]);
        expect(container.read(presenceCacheProvider), hasLength(207));
      });
    },
  );

  test('initial subscription failure retries without snapshotting a gap', () {
    fakeAsync((time) {
      final relay = PresenceTestRelay()..failSubscribe = true;
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => relay),
          appLifecycleProvider.overrideWith(_Lifecycle.new),
        ],
      );
      container.read(presenceCacheProvider.notifier).track(['alice']);
      time.elapse(Duration.zero);
      expect(relay.queries, isEmpty);
      expect(container.read(presenceCacheProvider), isEmpty);
      relay.failSubscribe = false;
      time.elapse(const Duration(seconds: 60));
      expect(relay.subscriptions, 2);
      expect(relay.queries.single.authors, ['alice']);
      container.dispose();
    });
  });

  test(
    'subscribe first; dispose before ready closes the late subscription',
    () {
      fakeAsync((time) {
        final relay = PresenceTestRelay()..ready = Completer<void>();
        final container = ProviderContainer(
          overrides: [
            relaySessionProvider.overrideWith(() => relay),
            appLifecycleProvider.overrideWith(_Lifecycle.new),
          ],
        );
        container.read(presenceCacheProvider.notifier).track(['alice']);
        time.elapse(Duration.zero);
        expect(relay.queries, isEmpty);
        container.dispose();
        relay.ready!.complete();
        time.flushMicrotasks();
        expect(relay.unsubscribes, 1);
        expect(relay.queries, isEmpty);
      });
    },
  );
}

NostrEvent presenceEvent(
  String author,
  String status, {
  String? subject,
  int timestamp = 10,
}) => NostrEvent(
  id: '$author-$status-$timestamp-$subject',
  pubkey: author,
  createdAt: timestamp,
  kind: EventKind.presenceUpdate,
  tags: [
    if (subject != null) ['p', subject],
  ],
  content: status,
  sig: 'sig',
);

class _Lifecycle extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

class PresenceTestRelay extends RelaySessionNotifier {
  final queries = <NostrFilter>[];
  final results = <Completer<List<NostrEvent>>>[];
  late void Function(NostrEvent) emit;
  late void Function(RelaySubscriptionStatus) status;
  late void Function(String) close;
  Completer<void>? ready;
  bool failSubscribe = false;
  bool emptySnapshots = false;
  int subscriptions = 0;
  int unsubscribes = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String)? onClosed,
    required void Function(RelaySubscriptionStatus) onStatusChanged,
  }) async {
    if (!filter.kinds.contains(EventKind.presenceUpdate)) {
      onStatusChanged(RelaySubscriptionStatus.ready);
      return () {};
    }
    subscriptions++;
    if (failSubscribe) throw StateError('unavailable');
    emit = onEvent;
    status = onStatusChanged;
    close = onClosed!;
    onStatusChanged(RelaySubscriptionStatus.ready);
    if (ready != null) await ready!.future;
    return () => unsubscribes++;
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String)? onClosed,
  }) async => () {};

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    if (!filters.single.kinds.contains(EventKind.presenceUpdate)) {
      return Future.value([]);
    }
    queries.addAll(filters);
    if (emptySnapshots) return Future.value([]);
    final result = Completer<List<NostrEvent>>();
    results.add(result);
    return result.future;
  }
}
