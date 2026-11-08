import 'dart:async';

import 'package:buzz/features/profile/presence_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  void scenario(void Function(FakeAsync, ProviderContainer, _Relay) run) {
    fakeAsync((time) {
      final relay = _Relay();
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => relay),
          appLifecycleProvider.overrideWith(_Lifecycle.new),
        ],
      );
      container.read(presenceCacheProvider.notifier).track(['alice']);
      time.elapse(Duration.zero);
      try {
        run(time, container, relay);
      } finally {
        container.dispose();
        time.flushMicrotasks();
      }
    });
  }

  test('delayed pre-snapshot live event cannot regress a settled snapshot', () {
    scenario((time, container, relay) {
      relay.results.removeAt(0).complete([
        _event('relay', 'online', subject: 'alice', timestamp: 20),
      ]);
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider)['alice'], 'online');

      // Delivered after the snapshot settled but signed ten seconds before
      // the relay observed it: the observation already folded it in.
      relay.emit(_event('alice', 'offline', timestamp: 10));
      expect(container.read(presenceCacheProvider)['alice'], 'online');

      time.elapse(Duration.zero);
      expect(relay.queries, hasLength(2));
      expect(relay.queries.last.authors, ['alice']);
      relay.results.removeAt(0).complete([
        _event('relay', 'online', subject: 'alice', timestamp: 22),
      ]);
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider)['alice'], 'online');
    });
  });

  test('live event from a lagging signer clock is confirmed, not dropped', () {
    scenario((time, container, relay) {
      relay.results.removeAt(0).complete([
        _event('relay', 'online', subject: 'alice', timestamp: 200),
      ]);
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider)['alice'], 'online');

      // Signed behind the relay clock but ingested before fan-out, so the
      // fresh observation must carry its effect instead of trusting either
      // clock against the other.
      relay.emit(_event('alice', 'offline', timestamp: 190));
      expect(container.read(presenceCacheProvider)['alice'], 'online');

      time.elapse(Duration.zero);
      expect(relay.queries, hasLength(2));
      relay.results.removeAt(0).complete([
        _event('relay', 'offline', subject: 'alice', timestamp: 201),
      ]);
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider)['alice'], 'offline');
    });
  });

  for (final settled in ['online', 'offline']) {
    for (final newer in [false, true]) {
      test(
        'equal-second $settled / newer=$newer requires fresh confirmation',
        () {
          scenario((time, container, relay) {
            final transition = settled == 'online' ? 'offline' : 'online';
            relay.results.removeAt(0).complete([
              _event('relay', 'online', subject: 'alice', timestamp: 20),
            ]);
            time.flushMicrotasks();
            if (settled == 'offline') {
              time.elapse(const Duration(seconds: 60));
              relay.results.removeAt(0).complete([]);
              time.flushMicrotasks();
            }
            expect(container.read(presenceCacheProvider)['alice'], settled);
            relay.emit(_event('alice', transition, timestamp: 20));
            expect(container.read(presenceCacheProvider)['alice'], settled);
            time.elapse(Duration.zero);
            expect(relay.queries, hasLength(settled == 'online' ? 2 : 3));
            expect(relay.queries.last.authors, ['alice']);
            expect(container.read(presenceCacheProvider)['alice'], settled);
            // Same observation second, but this POST follows live delivery.
            final confirmed = newer ? transition : settled;
            relay.results.removeAt(0).complete([
              if (confirmed == 'online')
                _event('relay', 'online', subject: 'alice', timestamp: 20),
            ]);
            time.flushMicrotasks();
            expect(
              container.read(presenceCacheProvider)['alice'],
              newer ? transition : settled,
            );
          });
        },
      );
    }
  }

  test('redelivered ambiguous event arms one confirmation', () {
    scenario((time, container, relay) {
      relay.results.removeAt(0).complete([
        _event('relay', 'online', subject: 'alice', timestamp: 20),
      ]);
      time.flushMicrotasks();
      final delayed = _event('alice', 'offline', timestamp: 10);
      relay.emit(delayed);
      time.elapse(Duration.zero);
      relay.results.removeAt(0).complete([
        _event('relay', 'online', subject: 'alice', timestamp: 22),
      ]);
      time.flushMicrotasks();
      expect(container.read(presenceCacheProvider)['alice'], 'online');

      // The same frame is redelivered after the confirmation settled; a
      // bounded id memory must keep it from re-arming a refresh.
      relay.emit(delayed);
      time.elapse(Duration.zero);
      expect(relay.queries, hasLength(2));
      expect(container.read(presenceCacheProvider)['alice'], 'online');
    });
  });
}

NostrEvent _event(
  String author,
  String status, {
  String? subject,
  int timestamp = 10,
}) => NostrEvent(
  id: '$author-$status-$timestamp',
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

class _Relay extends RelaySessionNotifier {
  final queries = <NostrFilter>[];
  final results = <Completer<List<NostrEvent>>>[];
  late void Function(NostrEvent) emit;
  Completer<void>? ready;
  bool failSubscribe = false;
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
    subscriptions++;
    if (failSubscribe) throw StateError('unavailable');
    emit = onEvent;
    onStatusChanged(RelaySubscriptionStatus.ready);
    if (ready != null) await ready!.future;
    return () => unsubscribes++;
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    queries.addAll(filters);
    final result = Completer<List<NostrEvent>>();
    results.add(result);
    return result.future;
  }
}
