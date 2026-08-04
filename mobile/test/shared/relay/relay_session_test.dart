import 'dart:async';
import 'dart:convert';

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

  test('queryRelay arms the rate-limit gate from a 429 retry hint', () async {
    final gateTimers = <_ManualTimer>[];
    final gate = RelayRateLimitGate(
      now: () => DateTime(2026),
      timerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        gateTimers.add(timer);
        return timer;
      },
    );
    const body = '{"error":"rate-limited: quota exceeded; retry in 4s"}';
    final harness = _queryHarness(
      gate: gate,
      client: http_testing.MockClient((_) async => http.Response(body, 429)),
    );
    addTearDown(harness.container.dispose);

    await expectLater(
      harness.session.queryRelay(const []),
      throwsA(
        isA<RelayException>()
            .having((error) => error.statusCode, 'statusCode', 429)
            .having((error) => error.body, 'body', body),
      ),
    );

    expect(gateTimers.single.duration, const Duration(seconds: 4));
    expect(gate.isActive, isTrue);
  });

  test('queryRelay uses the default gate for a 503 without a hint', () async {
    final gateTimers = <_ManualTimer>[];
    final gate = RelayRateLimitGate(
      now: () => DateTime(2026),
      timerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        gateTimers.add(timer);
        return timer;
      },
    );
    const body = '{"error":"rate-limited: shared admission unavailable"}';
    final harness = _queryHarness(
      gate: gate,
      client: http_testing.MockClient((_) async => http.Response(body, 503)),
    );
    addTearDown(harness.container.dispose);

    await expectLater(
      harness.session.queryRelay(const []),
      throwsA(
        isA<RelayException>()
            .having((error) => error.statusCode, 'statusCode', 503)
            .having((error) => error.body, 'body', body),
      ),
    );

    expect(gateTimers.single.duration, const Duration(seconds: 10));
    expect(gate.isActive, isTrue);
  });

  test('queryRelay does not arm the gate for a non-rate-limit error', () async {
    final gateTimers = <_ManualTimer>[];
    final gate = RelayRateLimitGate(
      now: () => DateTime(2026),
      timerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        gateTimers.add(timer);
        return timer;
      },
    );
    const body = '{"error":"not found"}';
    final harness = _queryHarness(
      gate: gate,
      client: http_testing.MockClient((_) async => http.Response(body, 404)),
    );
    addTearDown(harness.container.dispose);

    await expectLater(
      harness.session.queryRelay(const []),
      throwsA(
        isA<RelayException>()
            .having((error) => error.statusCode, 'statusCode', 404)
            .having((error) => error.body, 'body', body),
      ),
    );

    expect(gateTimers, isEmpty);
    expect(gate.isActive, isFalse);
  });

  test('queryRelay preserves an error with an unrecognized body', () async {
    final gateTimers = <_ManualTimer>[];
    final gate = RelayRateLimitGate(
      now: () => DateTime(2026),
      timerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        gateTimers.add(timer);
        return timer;
      },
    );
    const body = 'upstream unavailable';
    final harness = _queryHarness(
      gate: gate,
      client: http_testing.MockClient((_) async => http.Response(body, 503)),
    );
    addTearDown(harness.container.dispose);

    await expectLater(
      harness.session.queryRelay(const []),
      throwsA(
        isA<RelayException>()
            .having((error) => error.statusCode, 'statusCode', 503)
            .having((error) => error.body, 'body', body),
      ),
    );

    expect(gateTimers, isEmpty);
    expect(gate.isActive, isFalse);
  });

  test('queryRelay success does not arm the rate-limit gate', () async {
    final gateTimers = <_ManualTimer>[];
    final gate = RelayRateLimitGate(
      now: () => DateTime(2026),
      timerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        gateTimers.add(timer);
        return timer;
      },
    );
    final harness = _queryHarness(
      gate: gate,
      client: http_testing.MockClient((_) async => http.Response('[]', 200)),
    );
    addTearDown(harness.container.dispose);

    expect(await harness.session.queryRelay(const []), isEmpty);
    expect(gateTimers, isEmpty);
    expect(gate.isActive, isFalse);
  });

  test('queryRelay does not wait for an active rate-limit gate', () async {
    final gate = RelayRateLimitGate(
      now: () => DateTime(2026),
      timerFactory: _ManualTimer.new,
    );
    var requestCount = 0;
    final harness = _queryHarness(
      gate: gate,
      client: http_testing.MockClient((_) async {
        requestCount++;
        return http.Response('[]', 200);
      }),
    );
    addTearDown(harness.container.dispose);
    // Let the provider's build/dispose churn settle before arming: reading the
    // notifier registers `ref.onDispose(_dispose)`, and `_dispose` resets the
    // shared gate. Arming before that settles leaves the gate disarmed by the
    // time the request runs, which makes this row pass for the wrong reason.
    await pumpEventQueue();
    gate.activate(4);
    expect(gate.isActive, isTrue);

    final query = harness.session.queryRelay(const []);
    await Future<void>.delayed(Duration.zero);

    expect(requestCount, 1);
    expect(await query, isEmpty);
    // Still armed: the read must neither wait on the gate nor clear it.
    expect(gate.isActive, isTrue);
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

  test(
    'resume reconnects a stale connected session after a long pause',
    () async {
      final sockets = <_ControlledRelaySocket>[];
      final keychain = nostr.Keys.generate();
      var now = DateTime(2026, 8, 2, 12);
      final session = RelaySessionNotifier(
        now: () => now,
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
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => session),
          relayConfigProvider.overrideWith(
            () => _FakeRelayConfigNotifier(
              baseUrl: 'https://relay.example',
              nsec: keychain.nsec,
            ),
          ),
          authProvider.overrideWith(() => _AuthenticatedAuthNotifier()),
        ],
      );
      addTearDown(container.dispose);
      await container.read(authProvider.future);
      final subscription = container.listen(relaySessionProvider, (_, _) {});
      addTearDown(subscription.close);
      await Future<void>.delayed(Duration.zero);
      sockets.single.connectSuccessfully();

      session.onAppPaused();
      now = now.add(const Duration(minutes: 5));
      session.onAppResumed();
      await Future<void>.delayed(Duration.zero);

      expect(sockets, hasLength(2));
      expect(sockets.first.disposeCalls, 1);
      expect(session.state.status, SessionStatus.reconnecting);
    },
  );

  test(
    'resume keeps a connected session within the background grace period',
    () async {
      final sockets = <_ControlledRelaySocket>[];
      final keychain = nostr.Keys.generate();
      var now = DateTime(2026, 8, 2, 12);
      final session = RelaySessionNotifier(
        now: () => now,
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
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => session),
          relayConfigProvider.overrideWith(
            () => _FakeRelayConfigNotifier(
              baseUrl: 'https://relay.example',
              nsec: keychain.nsec,
            ),
          ),
          authProvider.overrideWith(() => _AuthenticatedAuthNotifier()),
        ],
      );
      addTearDown(container.dispose);
      await container.read(authProvider.future);
      final subscription = container.listen(relaySessionProvider, (_, _) {});
      addTearDown(subscription.close);
      await Future<void>.delayed(Duration.zero);
      sockets.single.connectSuccessfully();

      session.onAppPaused();
      now = now.add(const Duration(seconds: 4));
      session.onAppResumed();
      await Future<void>.delayed(Duration.zero);

      expect(sockets, hasLength(1));
      expect(sockets.single.disposeCalls, 0);
      expect(session.state.status, SessionStatus.connected);
    },
  );

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

  test('flushes replay events before a post-EOSE query can begin', () async {
    final session = RelaySessionNotifier();
    final deliveryPhases = <bool>[];
    var queryHasBegun = false;
    const filter = NostrFilter(
      kinds: EventKind.channelEventKinds,
      tags: {
        '#h': [_channelId],
      },
      limit: 50,
    );

    final subscribe = session.subscribe(
      filter,
      (_) => deliveryPhases.add(queryHasBegun),
    );
    final replayEvent = _event();
    session.debugHandleMessage(['EVENT', 'l-1', replayEvent.toJson()]);
    session.debugHandleMessage(['EOSE', 'l-1']);

    final unsubscribe = await subscribe;
    queryHasBegun = true;
    // The original batch timer must not deliver the replay event after the
    // caller has advanced to its query phase.
    session.debugFlushEventBuffer();

    expect(deliveryPhases, [false]);
    unsubscribe();
  });

  test('terminal CLOSED fails a live subscribe before ready', () async {
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

  test(
    'retryable CLOSED before EOSE retains and retries the live sub',
    () async {
      final timers = <_ManualTimer>[];
      final socket = _RecordingRelaySocket();
      final session = RelaySessionNotifier(
        retryTimerFactory: (duration, callback) {
          final timer = _ManualTimer(duration, callback);
          timers.add(timer);
          return timer;
        },
      );
      session.debugAttachSocketForTest(socket);

      final subscribe = session.subscribe(_channelFilter, (_) {});
      session.debugHandleMessage(['CLOSED', 'l-1', 'error: relay overloaded']);
      final unsubscribe = await subscribe;

      expect(timers.single.duration, const Duration(seconds: 1));
      timers.single.fire();
      await Future<void>.delayed(Duration.zero);

      expect(_reqs(socket).where((req) => req[1] == 'l-1'), hasLength(2));
      unsubscribe();
    },
  );

  test('CLOSED retries back off and reset after EOSE', () async {
    final timers = <_ManualTimer>[];
    final socket = _RecordingRelaySocket();
    final session = RelaySessionNotifier(
      retryTimerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        timers.add(timer);
        return timer;
      },
    );
    session.debugAttachSocketForTest(socket);
    final subscribe = session.subscribe(_channelFilter, (_) {});
    session.debugHandleMessage(['EOSE', 'l-1']);
    final unsubscribe = await subscribe;

    session.debugHandleMessage(['CLOSED', 'l-1', 'error: transient']);
    expect(timers.last.duration, const Duration(seconds: 1));
    timers.last.fire();
    await Future<void>.delayed(Duration.zero);
    session.debugHandleMessage(['CLOSED', 'l-1', 'error: transient']);
    expect(timers.last.duration, const Duration(seconds: 2));

    session.debugHandleMessage(['EOSE', 'l-1']);
    session.debugHandleMessage(['CLOSED', 'l-1', 'error: transient']);
    expect(timers.last.duration, const Duration(seconds: 1));
    unsubscribe();
  });

  test('CLOSED retry backoff saturates before a high-attempt shift', () async {
    final timers = <_ManualTimer>[];
    final socket = _RecordingRelaySocket();
    final session = RelaySessionNotifier(
      retryTimerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        timers.add(timer);
        return timer;
      },
    );
    session.debugAttachSocketForTest(socket);
    final subscribe = session.subscribe(_channelFilter, (_) {});
    session.debugHandleMessage(['EOSE', 'l-1']);
    final unsubscribe = await subscribe;

    for (var attempt = 0; attempt < 100; attempt++) {
      session.debugHandleMessage(['CLOSED', 'l-1', 'error: transient']);
      expect(
        timers.last.duration,
        attempt >= 5
            ? const Duration(seconds: 30)
            : Duration(seconds: 1 << attempt),
      );
      timers.last.fire();
      await Future<void>.delayed(Duration.zero);
    }

    unsubscribe();
  });

  test('CLOSED retries reset after a delivered event', () async {
    final timers = <_ManualTimer>[];
    final socket = _RecordingRelaySocket();
    final session = RelaySessionNotifier(
      retryTimerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        timers.add(timer);
        return timer;
      },
    );
    session.debugAttachSocketForTest(socket);
    final subscribe = session.subscribe(_channelFilter, (_) {});
    session.debugHandleMessage(['EOSE', 'l-1']);
    final unsubscribe = await subscribe;

    session.debugHandleMessage(['CLOSED', 'l-1', 'error: transient']);
    timers.last.fire();
    await Future<void>.delayed(Duration.zero);
    session.debugHandleMessage(['CLOSED', 'l-1', 'error: transient']);
    expect(timers.last.duration, const Duration(seconds: 2));

    session.debugHandleMessage([
      'EVENT',
      'l-1',
      _event(createdAt: 30).toJson(),
    ]);
    session.debugHandleMessage(['CLOSED', 'l-1', 'error: transient']);
    expect(timers.last.duration, const Duration(seconds: 1));
    unsubscribe();
  });

  test('CLOSED retries reset after disconnect and reconnect', () async {
    final timers = <_ManualTimer>[];
    final socket = _RecordingRelaySocket();
    final session = RelaySessionNotifier(
      retryTimerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        timers.add(timer);
        return timer;
      },
    );
    session.debugAttachSocketForTest(socket);
    final subscribe = session.subscribe(_channelFilter, (_) {});
    session.debugHandleMessage(['EOSE', 'l-1']);
    final unsubscribe = await subscribe;

    session.debugHandleMessage(['CLOSED', 'l-1', 'error: transient']);
    timers.last.fire();
    await Future<void>.delayed(Duration.zero);
    session.debugHandleMessage(['CLOSED', 'l-1', 'error: transient']);
    expect(timers.last.duration, const Duration(seconds: 2));

    session.debugResetClosedRetriesForDisconnect();
    expect(timers.last.isActive, isFalse);
    session.debugSetSessionStatus(SessionStatus.connected);
    session.debugHandleMessage(['CLOSED', 'l-1', 'error: transient']);
    expect(timers.last.duration, const Duration(seconds: 1));
    unsubscribe();
  });

  test('a CLOSED retry timer does not send while disconnected', () async {
    final timers = <_ManualTimer>[];
    final socket = _RecordingRelaySocket();
    final session = RelaySessionNotifier(
      retryTimerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        timers.add(timer);
        return timer;
      },
    );
    session.debugAttachSocketForTest(socket);
    final subscribe = session.subscribe(_channelFilter, (_) {});
    session.debugHandleMessage(['EOSE', 'l-1']);
    final unsubscribe = await subscribe;
    final requestCount = _reqs(socket).length;

    session.debugHandleMessage(['CLOSED', 'l-1', 'error: transient']);
    session.debugSetSessionStatus(SessionStatus.reconnecting);
    timers.single.fire();
    await Future<void>.delayed(Duration.zero);

    expect(_reqs(socket), hasLength(requestCount));
    unsubscribe();
  });

  test('terminal CLOSED removes a live sub without retrying it', () async {
    final timers = <_ManualTimer>[];
    final socket = _RecordingRelaySocket();
    final session = RelaySessionNotifier(
      retryTimerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        timers.add(timer);
        return timer;
      },
    );
    session.debugAttachSocketForTest(socket);
    final subscribe = session.subscribe(_channelFilter, (_) {});
    session.debugHandleMessage(['EOSE', 'l-1']);
    await subscribe;

    session.debugHandleMessage(['CLOSED', 'l-1', 'restricted: access revoked']);
    await session.debugReplayLiveSubscriptions();

    expect(timers, isEmpty);
    expect(_reqs(socket).where((req) => req[1] == 'l-1'), hasLength(1));
  });

  test('unsubscribe and dispose cancel CLOSED retry timers', () async {
    final timers = <_ManualTimer>[];
    final socket = _RecordingRelaySocket();
    final session = RelaySessionNotifier(
      retryTimerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        timers.add(timer);
        return timer;
      },
    );
    session.debugAttachSocketForTest(socket);

    final firstSubscribe = session.subscribe(_channelFilter, (_) {});
    session.debugHandleMessage(['EOSE', 'l-1']);
    final unsubscribe = await firstSubscribe;
    session.debugHandleMessage(['CLOSED', 'l-1', 'error: transient']);
    final unsubscribeTimer = timers.last;
    unsubscribe();
    expect(unsubscribeTimer.isActive, isFalse);

    final secondSubscribe = session.subscribe(_channelFilter, (_) {});
    session.debugHandleMessage(['EOSE', 'l-2']);
    await secondSubscribe;
    session.debugHandleMessage(['CLOSED', 'l-2', 'error: transient']);
    final disposeTimer = timers.last;
    session.debugDispose();
    expect(disposeTimer.isActive, isFalse);
  });

  test('rate-limited live CLOSED honours the gate floor', () async {
    final retryTimers = <_ManualTimer>[];
    final gateTimers = <_ManualTimer>[];
    final gate = RelayRateLimitGate(
      timerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        gateTimers.add(timer);
        return timer;
      },
    );
    final session = RelaySessionNotifier(
      rateLimitGate: gate,
      retryTimerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        retryTimers.add(timer);
        return timer;
      },
    );
    final socket = _RecordingRelaySocket();
    session.debugAttachSocketForTest(socket);
    final subscribe = session.subscribe(_channelFilter, (_) {});
    session.debugHandleMessage(['EOSE', 'l-1']);
    final unsubscribe = await subscribe;

    session.debugHandleMessage([
      'CLOSED',
      'l-1',
      'rate-limited: quota exceeded; retry in 4s',
    ]);

    expect(
      retryTimers.single.duration.inMilliseconds,
      inInclusiveRange(3990, 4000),
    );
    expect(gateTimers.single.duration, const Duration(seconds: 4));
    unsubscribe();
  });

  test(
    'rate-limited CLOSED retry does not survive a superseded connection',
    () async {
      final retryTimers = <_ManualTimer>[];
      final gateTimers = <_ManualTimer>[];
      final gate = RelayRateLimitGate(
        now: () => DateTime(2026),
        timerFactory: (duration, callback) {
          final timer = _ManualTimer(duration, callback);
          gateTimers.add(timer);
          return timer;
        },
      );
      final socket = _RecordingRelaySocket();
      final session = RelaySessionNotifier(
        rateLimitGate: gate,
        retryTimerFactory: (duration, callback) {
          final timer = _ManualTimer(duration, callback);
          retryTimers.add(timer);
          return timer;
        },
      );
      session.debugAttachSocketForTest(socket);
      final subscribe = session.subscribe(_channelFilter, (_) {});
      session.debugHandleMessage(['EOSE', 'l-1']);
      final unsubscribe = await subscribe;
      socket.messages.clear();

      session.debugHandleMessage([
        'CLOSED',
        'l-1',
        'rate-limited: quota exceeded; retry in 4s',
      ]);
      retryTimers.single.fire();
      await Future<void>.delayed(Duration.zero);
      expect(_reqs(socket), isEmpty);

      session.debugSupersedeConnection();
      final replacementReplay = session.debugReplayLiveSubscriptions();
      await Future<void>.delayed(Duration.zero);
      gateTimers.single.fire();
      await replacementReplay;
      await Future<void>.delayed(Duration.zero);

      expect(_reqs(socket).where((req) => req[1] == 'l-1'), hasLength(1));
      unsubscribe();
    },
  );

  test('simultaneous rate-limited CLOSED retries are replay-paced', () async {
    final retryTimers = <_ManualTimer>[];
    final gateTimers = <_ManualTimer>[];
    final replayDelays = <Duration>[];
    final replayDelayCompleters = <Completer<void>>[];
    final gate = RelayRateLimitGate(
      now: () => DateTime(2026),
      timerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        gateTimers.add(timer);
        return timer;
      },
    );
    final socket = _RecordingRelaySocket();
    final session = RelaySessionNotifier(
      rateLimitGate: gate,
      retryTimerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        retryTimers.add(timer);
        return timer;
      },
      replayDelay: (duration) {
        replayDelays.add(duration);
        final completer = Completer<void>();
        replayDelayCompleters.add(completer);
        return completer.future;
      },
    );
    session.debugAttachSocketForTest(socket);

    for (var i = 0; i < 30; i++) {
      final subscribe = session.subscribe(
        _filterForChannel('channel-$i'),
        (_) {},
      );
      session.debugHandleMessage(['EOSE', 'l-${i + 1}']);
      await subscribe;
    }
    socket.messages.clear();

    for (var i = 0; i < 30; i++) {
      session.debugHandleMessage([
        'CLOSED',
        'l-${i + 1}',
        'rate-limited: quota exceeded; retry in 4s',
      ]);
    }
    for (final timer in retryTimers) {
      timer.fire();
    }
    await Future<void>.delayed(Duration.zero);
    expect(_reqs(socket), isEmpty);

    gateTimers.single.fire();
    await Future<void>.delayed(Duration.zero);
    expect(_reqs(socket), hasLength(8));
    expect(replayDelays, [const Duration(milliseconds: 50)]);

    for (final expectedCount in [16, 24, 30]) {
      replayDelayCompleters.last.complete();
      await Future<void>.delayed(Duration.zero);
      expect(_reqs(socket), hasLength(expectedCount));
    }
    expect(replayDelays, [
      const Duration(milliseconds: 50),
      const Duration(milliseconds: 50),
      const Duration(milliseconds: 50),
    ]);
    session.debugDispose();
  });

  test('active rate-limit gate does not delay a new live subscribe', () async {
    final gateTimers = <_ManualTimer>[];
    final gate = RelayRateLimitGate(
      timerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        gateTimers.add(timer);
        return timer;
      },
    );
    final socket = _RecordingRelaySocket();
    final session = RelaySessionNotifier(rateLimitGate: gate);
    session.debugAttachSocketForTest(socket);
    gate.activate(4);

    final subscribe = session.subscribe(_channelFilter, (_) {});

    expect(_reqs(socket), hasLength(1));
    expect(gateTimers.single.duration, const Duration(seconds: 4));
    session.debugHandleMessage(['EOSE', 'l-1']);
    final unsubscribe = await subscribe;
    unsubscribe();
    session.debugDispose();
  });

  test('rate-limited history CLOSED gates the next REQ', () async {
    final gateTimers = <_ManualTimer>[];
    final gate = RelayRateLimitGate(
      timerFactory: (duration, callback) {
        final timer = _ManualTimer(duration, callback);
        gateTimers.add(timer);
        return timer;
      },
    );
    final socket = _RecordingRelaySocket();
    final session = RelaySessionNotifier(rateLimitGate: gate);
    session.debugAttachSocketForTest(socket);

    final first = session.fetchHistory(_channelFilter);
    session.debugHandleMessage([
      'CLOSED',
      'h-1',
      'rate-limited: quota exceeded; retry in 4s',
    ]);
    await expectLater(first, throwsException);

    final second = session.fetchHistory(_channelFilter);
    await Future<void>.delayed(Duration.zero);
    expect(_reqs(socket), hasLength(1));
    expect(gateTimers.single.duration, const Duration(seconds: 4));

    gateTimers.single.fire();
    await Future<void>.delayed(Duration.zero);
    expect(_reqs(socket), hasLength(2));
    session.debugHandleMessage(['EOSE', 'h-2']);
    await second;
  });

  test(
    'visible channel owners restore and ignore out-of-order release',
    () async {
      final socket = _RecordingRelaySocket();
      final session = RelaySessionNotifier();
      session.debugAttachSocketForTest(socket);
      const channelIds = ['channel-a', 'channel-b', 'channel-c'];

      for (var i = 0; i < channelIds.length; i++) {
        final subscribe = session.subscribe(
          _filterForChannel(channelIds[i]),
          (_) {},
        );
        session.debugHandleMessage(['EOSE', 'l-${i + 1}']);
        await subscribe;
      }

      final releaseA = session.registerVisibleChannel('channel-a');
      final releaseB = session.registerVisibleChannel('channel-b');
      final releaseC = session.registerVisibleChannel('channel-c');
      releaseB();
      socket.messages.clear();
      await session.debugReplayLiveSubscriptions();
      expect(_replayedChannelIds(socket).first, 'channel-c');

      releaseC();
      socket.messages.clear();
      await session.debugReplayLiveSubscriptions();
      expect(_replayedChannelIds(socket).first, 'channel-a');

      releaseB();
      releaseA();
    },
  );

  test('replay is visible-first and batched eight at a time', () async {
    final replayDelays = <Duration>[];
    final replayDelayCompleter = Completer<void>();
    final socket = _RecordingRelaySocket();
    final session = RelaySessionNotifier(
      replayDelay: (duration) {
        replayDelays.add(duration);
        return replayDelayCompleter.future;
      },
    );
    session.debugAttachSocketForTest(socket);

    for (var i = 0; i < 9; i++) {
      final channelId = i == 8 ? _visibleChannelId : 'channel-$i';
      final subscribe = session.subscribe(_filterForChannel(channelId), (_) {});
      session.debugHandleMessage(['EOSE', 'l-${i + 1}']);
      await subscribe;
    }
    socket.messages.clear();
    final releaseVisibleChannel = session.registerVisibleChannel(
      _visibleChannelId,
    );

    final replay = session.debugReplayLiveSubscriptions();
    await Future<void>.delayed(Duration.zero);

    final firstBatch = _reqs(socket);
    expect(firstBatch, hasLength(8));
    expect((firstBatch.first[2] as Map<String, dynamic>)['#h'], [
      _visibleChannelId,
    ]);
    expect(replayDelays, [const Duration(milliseconds: 50)]);

    replayDelayCompleter.complete();
    await replay;
    expect(_reqs(socket), hasLength(9));
    releaseVisibleChannel();
  });

  test(
    'replay generation guard bails after a connection is superseded',
    () async {
      final replayDelayCompleter = Completer<void>();
      final socket = _RecordingRelaySocket();
      final session = RelaySessionNotifier(
        replayDelay: (_) => replayDelayCompleter.future,
      );
      session.debugAttachSocketForTest(socket);

      for (var i = 0; i < 9; i++) {
        final subscribe = session.subscribe(
          _filterForChannel('channel-$i'),
          (_) {},
        );
        session.debugHandleMessage(['EOSE', 'l-${i + 1}']);
        await subscribe;
      }
      socket.messages.clear();

      final replay = session.debugReplayLiveSubscriptions();
      await Future<void>.delayed(Duration.zero);
      expect(_reqs(socket), hasLength(8));

      session.debugSupersedeConnection();
      replayDelayCompleter.complete();
      await replay;

      expect(_reqs(socket), hasLength(8));
    },
  );

  test('live onClosed callback runs only for a terminal CLOSED', () async {
    final session = RelaySessionNotifier();
    final closedMessages = <String>[];
    const filter = NostrFilter(kinds: [EventKind.agentObserverFrame], limit: 0);

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
      'error: temporarily unavailable',
    ]);
    expect(closedMessages, isEmpty);
    session.debugHandleMessage([
      'CLOSED',
      'l-1',
      'restricted: no longer valid',
    ]);

    expect(closedMessages, ['restricted: no longer valid']);
    unsubscribe();
  });
}

class _QueryHarness {
  final ProviderContainer container;
  final RelaySessionNotifier session;

  _QueryHarness({required this.container, required this.session});
}

_QueryHarness _queryHarness({
  required RelayRateLimitGate gate,
  required http.Client client,
}) {
  final session = RelaySessionNotifier(httpClient: client, rateLimitGate: gate);
  final container = ProviderContainer(
    overrides: [
      relaySessionProvider.overrideWith(() => session),
      relayConfigProvider.overrideWith(
        () => _FakeRelayConfigNotifier(
          baseUrl: 'https://relay.example',
          nsec: nostr.Keys.generate().nsec,
        ),
      ),
    ],
  );
  container.read(relaySessionProvider);
  return _QueryHarness(container: container, session: session);
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
  int disposeCalls = 0;

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
  void dispose() {
    disposeCalls++;
  }

  void connectSuccessfully() => _connected();

  void disconnectWith(Object? error) => _disconnected(error);
}

const _channelId = '11111111-1111-4111-8111-111111111111';

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  final String _baseUrl;
  final String? _nsec;

  _FakeRelayConfigNotifier({required String baseUrl, required String? nsec})
    : _baseUrl = baseUrl,
      _nsec = nsec;

  @override
  RelayConfig build() => RelayConfig(baseUrl: _baseUrl, nsec: _nsec);
}

NostrEvent _event({int createdAt = 20}) {
  return NostrEvent(
    id: 'event-1',
    pubkey: 'alice',
    createdAt: createdAt,
    kind: EventKind.streamMessageV2,
    tags: [
      ['h', _channelId],
    ],
    content: 'hello',
    sig: 'sig',
  );
}

const _visibleChannelId = '99999999-9999-4999-8999-999999999999';
const _channelFilter = NostrFilter(
  kinds: EventKind.channelEventKinds,
  tags: {
    '#h': [_channelId],
  },
  limit: 0,
);

NostrFilter _filterForChannel(String channelId) => NostrFilter(
  kinds: EventKind.channelEventKinds,
  tags: {
    '#h': [channelId],
  },
  limit: 0,
);

List<String> _replayedChannelIds(_RecordingRelaySocket socket) => _reqs(socket)
    .map(
      (message) =>
          ((message[2] as Map<String, dynamic>)['#h'] as List).single as String,
    )
    .toList();

List<List<dynamic>> _reqs(_RecordingRelaySocket socket) =>
    socket.messages.where((message) => message.first == 'REQ').toList();

class _RecordingRelaySocket extends RelaySocket {
  _RecordingRelaySocket()
    : super(
        wsUrl: 'wss://relay.example',
        nsec: null,
        onMessage: (_) {},
        onConnected: () {},
        onDisconnected: (_) {},
      );

  final List<List<dynamic>> messages = [];

  @override
  void send(List<dynamic> payload) => messages.add(payload);

  @override
  void dispose() {}
}

class _ManualTimer implements Timer {
  _ManualTimer(this.duration, this._callback);

  final Duration duration;
  final void Function() _callback;
  bool _active = true;

  void fire() {
    if (!_active) return;
    _active = false;
    _callback();
  }

  @override
  void cancel() => _active = false;

  @override
  bool get isActive => _active;

  @override
  int get tick => _active ? 0 : 1;
}
