import 'package:buzz/shared/relay/relay.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/fake_relay.dart';

void main() {
  // One step past the probe's wait.
  const pastProbe = Duration(milliseconds: 5100);

  ({FakeRelay relay, ProviderContainer container}) connect(FakeAsync async) {
    final relay = FakeRelay();
    final container = ProviderContainer(overrides: relay.overrides);
    addTearDown(container.dispose);
    container.listen(relaySessionProvider, (_, _) {});
    async.elapse(const Duration(milliseconds: 50));
    expect(
      container.read(relaySessionProvider).status,
      SessionStatus.connected,
    );
    return (relay: relay, container: container);
  }

  Iterable<Map<String, dynamic>> probes(FakeRelay relay) => relay.reqs
      .expand((req) => req.skip(2).cast<Map<String, dynamic>>())
      .where((filter) => filter['limit'] == 0 && filter['ids'] != null);

  test('a socket that stops answering after a network change is replaced', () {
    fakeAsync((async) {
      final (:relay, :container) = connect(async);
      final session = container.read(relaySessionProvider.notifier);

      // The socket stays open but nothing comes back, as one left on the
      // previous network does.
      relay.holdReq();
      session.onNetworkChanged();
      expect(probes(relay).single['kinds'], [
        13534,
      ], reason: 'a membership-list filter skips the channel access lookup');
      async.elapse(const Duration(milliseconds: 4900));
      expect(relay.sockets, hasLength(1), reason: 'still within the wait');

      async.elapse(const Duration(milliseconds: 200));
      expect(relay.sockets.first.isOpen, isFalse, reason: 'let go at once');
      // The reconnect starts after a random share of the spread, here half,
      // so a shared outage does not reconnect everyone at the same instant.
      async.elapse(const Duration(milliseconds: 1300));
      expect(relay.sockets, hasLength(1));
      async.elapse(const Duration(milliseconds: 200));
      expect(relay.sockets, hasLength(2));
      async.elapse(const Duration(milliseconds: 50));
      expect(
        container.read(relaySessionProvider).status,
        SessionStatus.connected,
      );
    });
  });

  test('a socket that answers is kept, and the probe is closed', () {
    fakeAsync((async) {
      final (:relay, :container) = connect(async);
      final session = container.read(relaySessionProvider.notifier);

      session.onNetworkChanged();
      async.elapse(const Duration(milliseconds: 50));
      expect(probes(relay), hasLength(1));
      expect(relay.liveFilters.expand((filters) => filters), isEmpty);

      // A second change while connected sends a new probe; neither ends it.
      session.onNetworkChanged();
      async.elapse(pastProbe * 2);
      expect(probes(relay), hasLength(2));
      expect(relay.sockets, hasLength(1));
      expect(
        container.read(relaySessionProvider).status,
        SessionStatus.connected,
      );
    });
  });

  test('a timed-out history request checks the socket, then backs off', () {
    fakeAsync((async) {
      final (:relay, :container) = connect(async);
      final session = container.read(relaySessionProvider.notifier);

      relay.holdReq();
      Object? failure;
      session
          .fetchHistory(const NostrFilter(kinds: [1], limit: 1))
          .then<void>((_) {}, onError: (Object error) => failure = error);
      async.elapse(const Duration(seconds: 8));
      expect(failure, isNotNull);
      expect(probes(relay), hasLength(1));

      // The relay may be overloaded, so the reconnect waits out the usual
      // backoff, at least 750 ms, instead of adding load at once.
      async.elapse(pastProbe);
      expect(relay.sockets, hasLength(1));
      expect(
        container.read(relaySessionProvider).status,
        SessionStatus.reconnecting,
      );
      async.elapse(const Duration(milliseconds: 1300));
      expect(relay.sockets, hasLength(2));
    });
  });

  test('a network change ends the reconnect backoff wait', () {
    fakeAsync((async) {
      final (:relay, :container) = connect(async);
      final session = container.read(relaySessionProvider.notifier);

      relay.drop();
      async.flushMicrotasks();
      expect(
        container.read(relaySessionProvider).status,
        SessionStatus.reconnecting,
      );
      expect(relay.sockets, hasLength(1), reason: 'waiting out the backoff');

      session.onNetworkChanged();
      async.elapse(const Duration(milliseconds: 50));
      expect(relay.sockets, hasLength(2));
      expect(
        container.read(relaySessionProvider).status,
        SessionStatus.connected,
      );
    });
  });

  test('a network change replaces a connection attempt in flight', () {
    fakeAsync((async) {
      final (:relay, :container) = connect(async);
      final session = container.read(relaySessionProvider.notifier);

      // The attempt after a drop is stuck in AUTH on the old network.
      relay.holdAuth();
      relay.drop();
      async.elapse(const Duration(seconds: 2));
      expect(relay.sockets, hasLength(2));
      expect(
        container.read(relaySessionProvider).status,
        SessionStatus.reconnecting,
      );

      session.onNetworkChanged();
      async.elapse(const Duration(milliseconds: 50));
      expect(relay.sockets, hasLength(3));
      expect(
        container.read(relaySessionProvider).status,
        SessionStatus.connected,
      );
    });
  });

  test('a refusal answers the probe as well as an end of results does', () {
    fakeAsync((async) {
      final (:relay, :container) = connect(async);
      final session = container.read(relaySessionProvider.notifier);

      relay.refuseReq((filter) => filter['limit'] == 0, 'blocked: no');
      session.onNetworkChanged();
      async.elapse(pastProbe);
      expect(relay.sockets, hasLength(1));
      expect(relay.closes, isEmpty, reason: 'the relay already closed it');
      expect(
        container.read(relaySessionProvider).status,
        SessionStatus.connected,
      );
    });
  });

  test('a network change during a pending probe sends a fresh one', () {
    fakeAsync((async) {
      final (:relay, :container) = connect(async);
      final session = container.read(relaySessionProvider.notifier);

      relay.holdReq();
      session
          .fetchHistory(const NostrFilter(kinds: [1], limit: 1))
          .then<void>((_) {}, onError: (_) {});
      async.elapse(const Duration(seconds: 8));
      expect(probes(relay), hasLength(1));
      session.onNetworkChanged();
      expect(probes(relay), hasLength(2));

      // It expires as a network change does: within the spread, not after
      // the backoff a timeout earns.
      async.elapse(pastProbe + relayNetworkChangeReconnectSpread);
      expect(relay.sockets, hasLength(2));
    });
  });

  test('a late close from a socket found dead changes nothing', () {
    fakeAsync((async) {
      final (:relay, :container) = connect(async);
      final session = container.read(relaySessionProvider.notifier);

      relay.holdReq();
      session
          .fetchHistory(const NostrFilter(kinds: [1], limit: 1))
          .then<void>((_) {}, onError: (_) {});
      async.elapse(const Duration(seconds: 8) + pastProbe);
      final dead = relay.sockets.single;
      expect(dead.isOpen, isFalse);
      final attempt = container.read(relaySessionProvider).reconnectAttempt;

      dead.closeLate(Exception('closed late'));
      async.flushMicrotasks();
      expect(
        container.read(relaySessionProvider).reconnectAttempt,
        attempt,
        reason: 'the backoff did not restart',
      );
    });
  });

  test('a pending probe ends with its session', () {
    fakeAsync((async) {
      final relay = FakeRelay();
      final container = ProviderContainer(overrides: relay.overrides);
      container.listen(relaySessionProvider, (_, _) {});
      async.elapse(const Duration(milliseconds: 50));
      relay.holdReq();
      container.read(relaySessionProvider.notifier).onNetworkChanged();
      container.dispose();
      expect(async.pendingTimers, isEmpty);
    });
  });
}
