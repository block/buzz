import 'dart:async';
import 'dart:io';

import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../helpers/fake_relay.dart';
import '../../helpers/recording_diagnostics.dart';

const _connection = NetworkEndpoint.relayConnection;
const _query = NetworkEndpoint.relayQuery;
const _publish = NetworkEndpoint.relayPublish;

/// What the session counts for what the network and the relay do, with only
/// the socket and HTTP boundary fake. None of it is an error report.
void main() {
  void journey(
    String name,
    void Function(FakeAsync async, _Session s) body, {
    bool protectedRelay = false,
  }) {
    test(name, () {
      fakeAsync((async) {
        final session = _Session(async, protectedRelay: protectedRelay);
        addTearDown(session.container.dispose);
        body(async, session);
      });
    });
  }

  journey('connecting, losing and regaining the connection are counted', (
    async,
    s,
  ) {
    s.settle();
    expect(s.counts.outcomes, [(_connection, null)]);
    expect(s.counts.events.single.backend, 'relay.test');
    // How long it took to connect.
    expect(s.counts.events.single.duration, isNotNull);

    final losses = <(Object?, NetworkFailure)>[
      (const SocketException('Connection refused'), NetworkFailure.unreachable),
      (const HandshakeException('bad certificate'), NetworkFailure.insecure),
      (TimeoutException('Relay connection timed out'), NetworkFailure.timeout),
      (null, NetworkFailure.disconnected),
      (Exception('error: restarting'), NetworkFailure.relayError),
    ];
    for (final (error, _) in losses) {
      s.relay.drop(error);
      // Past the backoff, so it reconnects.
      s.settle(const Duration(seconds: 40));
    }

    expect(s.counts.outcomes, [
      (_connection, null),
      for (final (_, failure) in losses) ...[
        (_connection, failure),
        (NetworkEndpoint.relayReconnect, null),
        (_connection, null),
      ],
    ]);
    // Everything on the trail too, and nothing was reported as an error.
    expect(s.counts.breadcrumbs, hasLength(s.counts.events.length));
    expect(s.counts.errors, isEmpty);

    // A relay that refuses the member ends it for good, with no reconnect.
    s.relay.drop(const RelayAuthRejectedException('restricted: not a member'));
    s.settle(const Duration(seconds: 40));
    expect(s.counts.outcomes.skip(1 + losses.length * 3), [
      (_connection, NetworkFailure.authRefused),
    ]);
  });

  journey('queries and publishes are counted by what the relay answers', (
    async,
    s,
  ) {
    s.settle();
    s.counts.clear();

    s.run(() => s.session.fetchHistory(const NostrFilter(kinds: [1])));
    final held = s.relay.holdReq();
    s.run(
      () => s.session.fetchHistory(
        const NostrFilter(kinds: [2]),
        timeout: const Duration(seconds: 2),
      ),
    );
    s.settle(const Duration(seconds: 3));
    held.complete();
    s.settle();

    s.relay.refuseReq(
      (f) => (f['kinds'] as List).contains(3),
      'restricted: no',
    );
    s.run(() => s.session.fetchHistory(const NostrFilter(kinds: [3])));
    s.settle();

    s.run(() => s.session.publish(s.relay.sign(kind: 1, tags: const [])));
    s.settle();
    s.relay.rejectNext('blocked: not allowed');
    s.run(() => s.session.publish(s.relay.sign(kind: 1, tags: const [])));
    final okHeld = s.relay.holdOk();
    s.run(
      () => s.session.publish(
        s.relay.sign(kind: 1, tags: const []),
        timeout: const Duration(seconds: 2),
      ),
    );
    s.settle(const Duration(seconds: 3));
    okHeld.complete();
    s.settle();

    s.run(() => s.session.subscribe(const NostrFilter(kinds: [4]), (_) {}));
    s.settle();
    s.relay.closeLive(
      (f) => (f['kinds'] as List).contains(4),
      'error: too many subscriptions',
    );
    s.settle();

    // The relay's rate limit is last: it holds later requests back.
    s.relay.refuseReq(
      (f) => (f['kinds'] as List).contains(5),
      'rate-limited: slow down, retry in 1s',
    );
    s.run(() => s.session.fetchHistory(const NostrFilter(kinds: [5])));
    s.settle();

    expect(s.counts.outcomes, [
      (_query, null),
      (_query, NetworkFailure.timeout),
      (_query, NetworkFailure.relayRefused),
      (_publish, null),
      (_publish, NetworkFailure.relayRefused),
      (_publish, NetworkFailure.timeout),
      (NetworkEndpoint.relaySubscription, NetworkFailure.relayError),
      (_query, NetworkFailure.rateLimited),
    ]);
    expect(s.counts.events.every((e) => e.backend == 'relay.test'), isTrue);
    expect(s.counts.errors, isEmpty);
  });

  journey('operations lost with the socket are counted once each', (async, s) {
    s.settle();
    s.counts.clear();

    void sendAndHold() {
      s.relay.holdReq();
      s.run(() => s.session.fetchHistory(const NostrFilter(kinds: [1])));
      s.relay.holdOk();
      s.run(() => s.session.publish(s.relay.sign(kind: 1, tags: const [])));
      s.settle();
    }

    // A query and a publish already sent when the connection drops.
    sendAndHold();
    s.relay.drop();
    // Past the backoff and every timeout: nothing is counted a second time.
    s.settle(const Duration(seconds: 40));
    expect(s.counts.outcomes, [
      (_connection, NetworkFailure.disconnected),
      (_query, NetworkFailure.disconnected),
      (_publish, NetworkFailure.disconnected),
      (NetworkEndpoint.relayReconnect, null),
      (_connection, null),
    ]);
    expect(s.counts.errors, isEmpty);

    // Moving to the background ends them on purpose, which is not a failure.
    s.counts.clear();
    sendAndHold();
    s.session.onAppPaused();
    s.settle(const Duration(seconds: 40));
    expect(s.counts.outcomes, isEmpty);
    s.session.onAppResumed();
    s.settle(const Duration(seconds: 40));

    // Nor is switching community.
    s.counts.clear();
    sendAndHold();
    s.container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://other.test', nsec: s.relay.keys.nsec);
    s.settle(const Duration(seconds: 40));
    expect(s.counts.outcomes, [(_connection, null)]);
  });

  journey('a protected relay\'s refusals are counted before the renewal', (
    async,
    s,
  ) {
    // The first upgrade is refused after 2 s. Renewing the assertion works,
    // so the refusal would otherwise leave no trace.
    s.relay.holdAuth();
    s.settle(const Duration(seconds: 2));
    s.relay.drop(const WebSocketException('refused', 403));
    s.settle();
    expect(s.counts.outcomes, [
      (_connection, NetworkFailure.authRefused),
      (_connection, null),
    ]);
    expect(s.counts.events.first.duration, const Duration(seconds: 2));
    // The renewed attempt is its own measurement.
    expect(s.counts.events.last.duration, lessThan(const Duration(seconds: 1)));

    // When the renewal is refused too, both refusals are counted and the
    // session gives up.
    s.counts.clear();
    s.relay.holdAuth();
    s.run(() => s.session.reconnect());
    s.settle(const Duration(seconds: 3));
    s.relay.holdAuth();
    s.relay.drop(const WebSocketException('refused', 403));
    s.settle(const Duration(seconds: 1));
    s.relay.drop(const WebSocketException('refused', 403));
    s.settle(const Duration(seconds: 40));
    expect(s.counts.outcomes, [
      (NetworkEndpoint.relayReconnect, null),
      (_connection, NetworkFailure.authRefused),
      (_connection, NetworkFailure.authRefused),
    ]);
    expect(s.counts.events.skip(1).map((e) => e.duration), [
      const Duration(seconds: 3),
      const Duration(seconds: 1),
    ]);
    expect(s.session.state.status, SessionStatus.disconnected);
  }, protectedRelay: true);

  journey('the relay\'s authorization denial is counted as a refusal', (
    async,
    s,
  ) {
    s.settle();
    s.counts.clear();

    // An ordinary NOTICE says nothing about the close that follows.
    s.session.debugHandleMessage(['NOTICE', 'still here']);
    s.relay.drop();
    s.settle(const Duration(seconds: 40));

    // The denial an expired hosted session gets comes before its close.
    s.session.debugHandleMessage([
      'NOTICE',
      'restricted: authorization denied',
    ]);
    s.relay.drop();
    s.settle(const Duration(seconds: 40));

    // It describes one connection only.
    s.relay.drop();
    s.settle(const Duration(seconds: 40));

    expect(s.counts.outcomes, [
      (_connection, NetworkFailure.disconnected),
      (NetworkEndpoint.relayReconnect, null),
      (_connection, null),
      (_connection, NetworkFailure.authRefused),
      (NetworkEndpoint.relayReconnect, null),
      (_connection, null),
      (_connection, NetworkFailure.disconnected),
      (NetworkEndpoint.relayReconnect, null),
      (_connection, null),
    ]);
  });

  journey('a community switch\'s first connect is not a reconnect', (async, s) {
    s.settle();
    s.counts.clear();

    s.container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://other.test', nsec: s.relay.keys.nsec);
    s.settle();

    expect(s.counts.outcomes, [(_connection, null)]);
  });

  journey('another member\'s session counts nothing', (async, s) {
    s.settle();
    s.counts.clear();

    s.counts.setMember(nostr.Keys.generate().public);
    s.relay.drop();
    s.settle(const Duration(seconds: 40));

    expect(s.counts.outcomes, isEmpty);
    expect(s.counts.breadcrumbs, isEmpty);
  });
}

class _Session {
  _Session(this.async, {bool protectedRelay = false}) {
    container = ProviderContainer(
      overrides: [
        ...relay.overrides,
        if (protectedRelay)
          hostedRelayAuthProvider.overrideWithValue(_ProtectedAuth()),
        diagnosticReporterProvider.overrideWithValue(counts),
      ],
    );
    container.listen(relaySessionProvider, (_, _) {});
  }

  final FakeAsync async;
  // On the fake clock, so durations are the time that passed in the test.
  late final relay = FakeRelay(now: async.getClock(DateTime.utc(2026)).now);
  final counts = RecordingDiagnostics();
  late final ProviderContainer container;

  RelaySessionNotifier get session => relay.session;

  void settle([Duration time = const Duration(milliseconds: 100)]) =>
      async.elapse(time);

  /// Starts [action], whose failure is part of the journey, not the test's.
  void run(Future<Object?> Function() action) =>
      unawaited(action().then<void>((_) {}, onError: (_) {}));
}

/// A hosted relay that wants an assertion, which it always gets.
class _ProtectedAuth extends HostedRelayAuth {
  _ProtectedAuth() : super(BuilderlabAccountClient()) {
    addTearDown(() {
      dispose();
      account.dispose();
    });
  }

  @override
  bool protects(RelayConfig config) => true;

  @override
  Future<Map<String, String>> headers(
    RelayConfig config, {
    bool background = false,
    bool fresh = false,
  }) async => {'Nostr-Federated-Identity': 'assertion'};
}
