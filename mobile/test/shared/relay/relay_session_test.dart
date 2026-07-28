import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';
import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  test('queryRelay sends NIP-98 auth over POST /query', () async {
    final keychain = nostr.Keys.generate();
    final nsec = keychain.nsec;
    http.Request? capturedRequest;
    final client = http_testing.MockClient((request) async {
      capturedRequest = request;
      return http.Response('[]', 200);
    });
    final session = RelaySessionNotifier(httpClient: client);
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => session),
        relayConfigProvider.overrideWith(
          () => _FakeRelayConfigNotifier(
            baseUrl: 'https://relay.example/base',
            nsec: nsec,
          ),
        ),
      ],
    );
    addTearDown(container.dispose);

    const filter = NostrFilter(
      kinds: EventKind.channelTimelineContentKinds,
      tags: {
        '#h': [_channelId],
      },
      limit: 50,
      extensions: {
        'top_level': true,
        'include_summaries': true,
        'include_aux': true,
      },
    );

    await container.read(relaySessionProvider.notifier).queryRelay([filter]);

    expect(capturedRequest, isNotNull);
    expect(capturedRequest!.method, 'POST');
    expect(capturedRequest!.url.toString(), 'https://relay.example/query');
    expect(capturedRequest!.headers['Content-Type'], 'application/json');
    expect(jsonDecode(capturedRequest!.body), [filter.toJson()]);

    final authHeader = capturedRequest!.headers['Authorization'];
    expect(authHeader, isNotNull);
    expect(authHeader, startsWith('Nostr '));
    final encoded = authHeader!.substring('Nostr '.length);
    final decoded = utf8.decode(base64Url.decode(base64Url.normalize(encoded)));
    final authEvent = jsonDecode(decoded) as Map<String, dynamic>;
    final tags = (authEvent['tags'] as List<dynamic>)
        .map((tag) => (tag as List<dynamic>).cast<String>())
        .toList();
    final payloadHash = SHA256Digest()
        .process(utf8.encode(capturedRequest!.body))
        .map((byte) => byte.toRadixString(16).padLeft(2, '0'))
        .join();

    expect(authEvent['kind'], 27235);
    expect(authEvent['pubkey'], keychain.public);
    expect(
      tags,
      anyElement(equals(<String>['u', 'https://relay.example/query'])),
    );
    expect(tags, anyElement(equals(<String>['method', 'POST'])));
    expect(tags, anyElement(equals(<String>['payload', payloadHash])));
    expect(tags.any((tag) => tag.length == 2 && tag[0] == 'nonce'), isTrue);
  });

  test('queryRelay rejects malformed event arrays', () async {
    final keychain = nostr.Keys.generate();
    final session = RelaySessionNotifier(
      httpClient: http_testing.MockClient(
        (_) async => http.Response('[{}]', 200),
      ),
    );
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => session),
        relayConfigProvider.overrideWith(
          () => _FakeRelayConfigNotifier(
            baseUrl: 'https://relay.example',
            nsec: keychain.nsec,
          ),
        ),
      ],
    );
    addTearDown(container.dispose);

    await expectLater(
      container.read(relaySessionProvider.notifier).queryRelay(const []),
      throwsA(isA<FormatException>()),
    );
  });

  test(
    'history timeout rejects instead of returning partial empty data',
    () async {
      final session = RelaySessionNotifier();

      await expectLater(
        session.fetchHistory(
          const NostrFilter(kinds: [39002]),
          timeout: const Duration(milliseconds: 1),
        ),
        throwsA(isA<TimeoutException>()),
      );
    },
  );

  test('background disconnect rejects in-flight history', () async {
    final session = RelaySessionNotifier();
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    container.read(relaySessionProvider);

    final history = session.fetchHistory(
      const NostrFilter(kinds: [39002]),
      timeout: const Duration(seconds: 1),
    );
    final expectation = expectLater(history, throwsException);

    session.debugPauseNow();

    await expectation;
  });

  test('retries a dropped connected session without live subscriptions', () {
    final session = RelaySessionNotifier();
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    container.read(relaySessionProvider);

    session.debugHandleConnected();
    session.debugHandleDisconnected();

    expect(session.state.status, SessionStatus.reconnecting);
    expect(session.state.reconnectAttempt, 1);
  });

  test('classifies relay internal auth errors as transient', () {
    expect(
      classifyRelayAuthFailure(
        'error: internal error checking restriction state',
      ),
      isNot(isA<RelayAuthRejectedException>()),
    );
    expect(
      classifyRelayAuthFailure('restricted: access revoked'),
      isA<RelayAuthRejectedException>(),
    );
  });

  test(
    'stops reconnecting without deleting community after auth rejection',
    () async {
      final session = RelaySessionNotifier();
      final auth = _FakeAuthNotifier();
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => session),
          authProvider.overrideWith(() => auth),
        ],
      );
      addTearDown(container.dispose);
      container.read(relaySessionProvider);

      session.debugHandleDisconnected(
        const RelayAuthRejectedException('auth-required: verification failed'),
      );
      await Future<void>.delayed(Duration.zero);

      expect(session.state.status, SessionStatus.disconnected);
      expect(auth.signOutCount, 0);
    },
  );

  test('ignores callbacks from a socket replaced by a config change', () async {
    final sockets = <_ControlledRelaySocket>[];
    final keychain = nostr.Keys.generate();
    final session = RelaySessionNotifier(
      socketFactory:
          ({
            required wsUrl,
            required nsec,
            required onMessage,
            required onConnected,
            required onDisconnected,
          }) {
            final socket = _ControlledRelaySocket(
              wsUrl: wsUrl,
              nsec: nsec,
              onMessage: onMessage,
              onConnected: onConnected,
              onDisconnected: onDisconnected,
            );
            sockets.add(socket);
            return socket;
          },
    );
    final config = _FakeRelayConfigNotifier(
      baseUrl: 'https://old.example',
      nsec: keychain.nsec,
    );
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => session),
        relayConfigProvider.overrideWith(() => config),
        authProvider.overrideWith(() => _AuthenticatedAuthNotifier()),
      ],
    );
    addTearDown(container.dispose);
    await container.read(authProvider.future);
    final subscription = container.listen(relaySessionProvider, (_, _) {});
    addTearDown(subscription.close);
    await Future<void>.delayed(Duration.zero);

    config.update(baseUrl: 'https://new.example', nsec: keychain.nsec);
    await Future<void>.delayed(Duration.zero);
    expect(sockets, hasLength(2));

    sockets.first.disconnectWith(
      const RelayAuthRejectedException('blocked: stale community'),
    );
    sockets.first.connectSuccessfully();
    expect(session.state.status, SessionStatus.connecting);

    sockets.last.connectSuccessfully();
    expect(session.state.status, SessionStatus.connected);
  });

  test('does not schedule reconnects after background disconnect', () {
    final session = RelaySessionNotifier();
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    container.read(relaySessionProvider);

    session.debugHandleConnected();
    session.debugPauseNow();
    session.debugHandleDisconnected();

    expect(session.state.status, SessionStatus.disconnected);
  });

  test('delivers the same live event to each matching subscription', () async {
    final session = RelaySessionNotifier();
    final firstEvents = <NostrEvent>[];
    final secondEvents = <NostrEvent>[];
    const filter = NostrFilter(
      kinds: EventKind.channelEventKinds,
      tags: {
        '#h': [_channelId],
      },
      limit: 50,
    );

    final firstSubscribe = session.subscribe(filter, firstEvents.add);
    session.debugHandleMessage(['EOSE', 'l-1']);
    final unsubscribeFirst = await firstSubscribe;

    final secondSubscribe = session.subscribe(filter, secondEvents.add);
    session.debugHandleMessage(['EOSE', 'l-2']);
    final unsubscribeSecond = await secondSubscribe;

    final event = _event();
    session.debugHandleMessage(['EVENT', 'l-1', event.toJson()]);
    session.debugHandleMessage(['EVENT', 'l-2', event.toJson()]);
    session.debugFlushEventBuffer();

    expect(firstEvents.map((event) => event.id), [event.id]);
    expect(secondEvents.map((event) => event.id), [event.id]);

    session.debugHandleMessage(['EVENT', 'l-1', event.toJson()]);
    session.debugFlushEventBuffer();

    expect(firstEvents.map((event) => event.id), [event.id]);
    expect(secondEvents.map((event) => event.id), [event.id]);

    unsubscribeFirst();
    unsubscribeSecond();
  });

  test('live subscribe fails when relay closes before ready', () async {
    final session = RelaySessionNotifier();
    const filter = NostrFilter(kinds: [EventKind.agentObserverFrame], limit: 0);

    final subscribe = session.subscribe(filter, (_) {});
    session.debugHandleMessage([
      'CLOSED',
      'l-1',
      'restricted: p-gated events require #p matching your pubkey',
    ]);

    await expectLater(
      subscribe,
      throwsA(
        isA<Exception>().having(
          (error) => error.toString(),
          'message',
          contains('p-gated events require #p'),
        ),
      ),
    );
  });

  // ── Gap 3: reconnect backoff jitter ──────────────────────────────────────

  test('jitters the scheduled reconnect delay around the ladder position', () {
    final session = RelaySessionNotifier(random: Random(1234));
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    container.read(relaySessionProvider);

    session.debugHandleConnected();
    session.debugHandleDisconnected();

    final delay = session.debugLastReconnectDelay;
    expect(delay, isNotNull);
    // The ladder itself stays un-jittered at base...
    expect(session.debugReconnectDelayMs, 1000);
    // ...but the scheduled wait is randomised within +/-20% of it. A flat
    // doubling schedule would land on exactly 1000ms.
    expect(delay!.inMilliseconds, isNot(1000));
    expect(delay.inMilliseconds, inInclusiveRange(800, 1200));
  });

  test('keeps every jittered delay within +/-20% and does not repeat', () {
    final session = RelaySessionNotifier(random: Random(7));
    final samples = <int>{};

    for (var i = 0; i < 200; i++) {
      final delay = session.debugJitteredDelay(8000).inMilliseconds;
      expect(delay, inInclusiveRange(6400, 9600));
      samples.add(delay);
    }

    // A flat (un-jittered) schedule would collapse to the single value 8000.
    expect(samples.length, greaterThan(1));
    expect(samples, isNot(contains(8000)));
  });

  // ── Gap 4: backoff resets on stability, not on connect ───────────────────

  test('does not reset the backoff ladder on a short-lived connection', () {
    final session = _backoffSession();

    // Three connect/drop cycles that never reach the stability threshold. The
    // ladder must keep doubling instead of snapping back to base each time.
    session.debugHandleConnected();
    expect(session.debugStableConnectionArmed, isTrue);
    session.debugHandleDisconnected();
    // Dropping before the threshold disarms the pending reset.
    expect(session.debugStableConnectionArmed, isFalse);
    expect(session.debugReconnectDelayMs, 1000);

    session.debugFireReconnectTimer();
    expect(session.debugReconnectDelayMs, 2000);

    session.debugHandleConnected();
    session.debugHandleDisconnected();
    session.debugFireReconnectTimer();
    expect(session.debugReconnectDelayMs, 4000);

    session.debugHandleConnected();
    session.debugHandleDisconnected();
    session.debugFireReconnectTimer();
    expect(session.debugReconnectDelayMs, 8000);
  });

  test('resets the backoff ladder once a connection proves stable', () {
    final session = _backoffSession();

    session.debugHandleConnected();
    session.debugHandleDisconnected();
    session.debugFireReconnectTimer();
    session.debugHandleConnected();
    session.debugHandleDisconnected();
    session.debugFireReconnectTimer();
    expect(session.debugReconnectDelayMs, 4000);

    // This connection survives the stability window, earning the ladder back.
    session.debugHandleConnected();
    session.debugCompleteStableConnection();

    expect(session.debugReconnectDelayMs, 1000);
    expect(session.debugStableConnectionArmed, isFalse);
  });

  test('resets the backoff ladder immediately on app resume', () {
    final session = _backoffSession();

    session.debugHandleConnected();
    session.debugHandleDisconnected();
    session.debugFireReconnectTimer();
    expect(session.debugReconnectDelayMs, 2000);

    // Backgrounding is our own teardown, not relay trouble: a foregrounding
    // user must not wait out a ladder the relay never asked for.
    session.debugPauseNow();
    session.onAppResumed();

    expect(session.debugReconnectDelayMs, 1000);
  });

  // ── Gap 5: NOTICE frames feed backpressure ───────────────────────────────

  test('rate-limit gate rejects publish without sending the event', () async {
    final fixture = _recordingSession();
    fixture.session.debugHandleMessage([
      'NOTICE',
      'rate-limited: quota exceeded; retry in 12s',
    ]);

    await expectLater(
      fixture.session.publish(
        _event(),
        timeout: const Duration(milliseconds: 1),
      ),
      throwsA(
        isA<RelayRateLimitedException>().having(
          (error) => error.retryAfter,
          'retryAfter',
          greaterThan(Duration.zero),
        ),
      ),
    );
    expect(fixture.socket.sent, isEmpty);
  });

  test('rate-limit gate drops ephemeral raw sends', () {
    final fixture = _recordingSession();
    fixture.session.sendRaw(['EVENT', _event().toJson()]);
    expect(fixture.socket.sent, hasLength(1));
    fixture.socket.sent.clear();

    fixture.session.debugHandleMessage([
      'NOTICE',
      'rate-limited: quota exceeded; retry in 12s',
    ]);

    fixture.session.sendRaw(['EVENT', _event().toJson()]);

    expect(fixture.socket.sent, isEmpty);
  });

  test('rate-limit NOTICE arms a gate separate from reconnect backoff', () {
    final session = RelaySessionNotifier(random: Random(1));

    session.debugHandleMessage([
      'NOTICE',
      'rate-limited: quota exceeded; retry in 12s',
    ]);

    expect(session.debugRateLimitRemaining, isNotNull);
    expect(session.debugReconnectDelayMs, 1000);
  });

  test('floors a rate-limit NOTICE without a usable hint', () {
    final withoutHint = RelaySessionNotifier(random: Random(1));
    withoutHint.debugHandleMessage([
      'NOTICE',
      'rate-limited: too many concurrent requests',
    ]);
    expect(
      withoutHint.debugRateLimitRemaining!.inMilliseconds,
      inInclusiveRange(3900, 6000),
    );

    final zeroHint = RelaySessionNotifier(random: Random(1));
    zeroHint.debugHandleMessage([
      'NOTICE',
      'rate-limited: quota exceeded; retry in 0s',
    ]);
    expect(
      zeroHint.debugRateLimitRemaining!.inMilliseconds,
      inInclusiveRange(3900, 6000),
    );
  });

  test('does not clamp a long rate-limit hint to reconnect maximum', () {
    final session = RelaySessionNotifier(random: Random(1));

    session.debugHandleMessage([
      'NOTICE',
      'rate-limited: quota exceeded; retry in 60s',
    ]);

    // A 30s clamp with +/-20% jitter can be at most 36s. The unclamped 60s
    // hint is at least 48s, leaving a wide margin for test execution time.
    expect(session.debugRateLimitRemaining!.inMilliseconds, greaterThan(40000));
  });

  test('never shortens an existing rate-limit deadline', () {
    final session = RelaySessionNotifier(random: Random(1));

    session.debugHandleMessage([
      'NOTICE',
      'rate-limited: quota exceeded; retry in 30s',
    ]);
    final longDeadline = session.debugRateLimitDeadlineMs;

    session.debugHandleMessage([
      'NOTICE',
      'rate-limited: quota exceeded; retry in 1s',
    ]);

    expect(session.debugRateLimitDeadlineMs, longDeadline);
  });

  test('stable connection reset preserves the rate-limit deadline', () {
    final session = RelaySessionNotifier(random: Random(1));
    session.debugHandleMessage([
      'NOTICE',
      'rate-limited: quota exceeded; retry in 60s',
    ]);
    final deadline = session.debugRateLimitDeadlineMs;

    session.debugCompleteStableConnection();

    expect(session.debugReconnectDelayMs, 1000);
    expect(session.debugRateLimitDeadlineMs, deadline);
  });

  test('lazily clears the rate-limit deadline once it expires', () {
    var nowMs = 100;
    final session = RelaySessionNotifier(
      random: Random(1),
      rateLimitNowMs: () => nowMs,
    );
    session.debugHandleMessage([
      'NOTICE',
      'rate-limited: quota exceeded; retry in 5s',
    ]);
    final deadline = session.debugRateLimitDeadlineMs!;

    expect(session.debugRateLimitRemaining, isNotNull);

    nowMs = deadline;

    expect(session.debugRateLimitRemaining, isNull);
    expect(session.debugRateLimitDeadlineMs, isNull);
  });

  test('leaves rate gate and backoff untouched for other NOTICE frames', () {
    final session = RelaySessionNotifier(random: Random(1));

    session.debugHandleMessage(['NOTICE', 'server restarting shortly']);
    expect(session.debugReconnectDelayMs, 1000);
    expect(session.debugRateLimitRemaining, isNull);

    // Malformed NOTICE frames must not throw.
    session.debugHandleMessage(['NOTICE']);
    session.debugHandleMessage(['NOTICE', 42]);
    expect(session.debugReconnectDelayMs, 1000);
    expect(session.debugRateLimitRemaining, isNull);
  });

  test(
    'live onClosed callback runs when relay closes an open subscription',
    () async {
      final session = RelaySessionNotifier();
      final closedMessages = <String>[];
      const filter = NostrFilter(
        kinds: [EventKind.agentObserverFrame],
        limit: 0,
      );

      final subscribe = session.subscribe(
        filter,
        (_) {},
        onClosed: closedMessages.add,
      );
      session.debugHandleMessage(['EOSE', 'l-1']);
      final unsubscribe = await subscribe;
      session.debugHandleMessage([
        'CLOSED',
        'l-1',
        'restricted: no longer valid',
      ]);

      expect(closedMessages, ['restricted: no longer valid']);
      unsubscribe();
    },
  );
}

class _FakeAuthNotifier extends AuthNotifier {
  int signOutCount = 0;

  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.unauthenticated);

  @override
  Future<void> signOut() async {
    signOutCount++;
  }
}

class _AuthenticatedAuthNotifier extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.authenticated);
}

class _ControlledRelaySocket extends RelaySocket {
  final void Function() _connected;
  final void Function(Object? error) _disconnected;

  _ControlledRelaySocket({
    required super.wsUrl,
    required super.nsec,
    required super.onMessage,
    required super.onConnected,
    required super.onDisconnected,
  }) : _connected = onConnected,
       _disconnected = onDisconnected;

  @override
  Future<void> connect() async {}

  @override
  void dispose() {}

  void connectSuccessfully() => _connected();

  void disconnectWith(Object? error) => _disconnected(error);
}

class _RecordingRelaySocket extends RelaySocket {
  final List<List<dynamic>> sent = [];

  _RecordingRelaySocket()
    : super(
        wsUrl: 'wss://relay.example',
        nsec: null,
        onMessage: (_) {},
        onConnected: () {},
        onDisconnected: (_) {},
      );

  @override
  void send(List<dynamic> payload) => sent.add(payload);

  @override
  void dispose() {}
}

class _RecordingSessionFixture {
  final RelaySessionNotifier session;
  final _RecordingRelaySocket socket;

  _RecordingSessionFixture(this.session, this.socket);
}

_RecordingSessionFixture _recordingSession() {
  final session = RelaySessionNotifier(random: Random(1));
  final container = ProviderContainer(
    overrides: [relaySessionProvider.overrideWith(() => session)],
  );
  addTearDown(container.dispose);
  container.read(relaySessionProvider);
  final socket = _RecordingRelaySocket();
  session.debugAttachSocketForTest(socket);
  return _RecordingSessionFixture(session, socket);
}

const _channelId = '11111111-1111-4111-8111-111111111111';

/// A session whose reconnects attach a no-op socket, so the backoff ladder can
/// be driven through real connect/drop cycles without touching the network.
RelaySessionNotifier _backoffSession({int seed = 1}) {
  final keychain = nostr.Keys.generate();
  final session = RelaySessionNotifier(
    random: Random(seed),
    socketFactory:
        ({
          required wsUrl,
          required nsec,
          required onMessage,
          required onConnected,
          required onDisconnected,
        }) => _ControlledRelaySocket(
          wsUrl: wsUrl,
          nsec: nsec,
          onMessage: onMessage,
          onConnected: onConnected,
          onDisconnected: onDisconnected,
        ),
  );
  final container = ProviderContainer(
    overrides: [
      relaySessionProvider.overrideWith(() => session),
      relayConfigProvider.overrideWith(
        () => _FakeRelayConfigNotifier(
          baseUrl: 'https://relay.example',
          nsec: keychain.nsec,
        ),
      ),
    ],
  );
  addTearDown(container.dispose);
  container.read(relaySessionProvider);
  return session;
}

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  final String _baseUrl;
  final String? _nsec;

  _FakeRelayConfigNotifier({required String baseUrl, required String? nsec})
    : _baseUrl = baseUrl,
      _nsec = nsec;

  @override
  RelayConfig build() => RelayConfig(baseUrl: _baseUrl, nsec: _nsec);
}

NostrEvent _event() {
  return const NostrEvent(
    id: 'event-1',
    pubkey: 'alice',
    createdAt: 20,
    kind: EventKind.streamMessageV2,
    tags: [
      ['h', _channelId],
    ],
    content: 'hello',
    sig: 'sig',
  );
}
