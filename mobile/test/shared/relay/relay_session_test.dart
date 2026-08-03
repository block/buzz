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
        socketFactory: _controlledSocketFactory(sockets),
      );
      final container = _authenticatedContainer(session, keychain.nsec);
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
    'resume preserves a connected session within the background grace period',
    () async {
      final sockets = <_ControlledRelaySocket>[];
      final keychain = nostr.Keys.generate();
      var now = DateTime(2026, 8, 2, 12);
      final session = RelaySessionNotifier(
        now: () => now,
        socketFactory: _controlledSocketFactory(sockets),
      );
      final container = _authenticatedContainer(session, keychain.nsec);
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

  test(
    'queues a disconnected publish and dispatches it exactly once on reconnect',
    () async {
      final session = RelaySessionNotifier();
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => session)],
      );
      addTearDown(container.dispose);
      container.read(relaySessionProvider);
      final subscription = container.listen(relaySessionProvider, (_, _) {});
      addTearDown(subscription.close);
      await Future<void>.delayed(Duration.zero);

      final firstSocket = _testSocket();
      session.debugAttachSocketForTest(firstSocket);
      session.debugHandleDisconnected();

      final publish = session.publish(_event());
      expect(firstSocket.sentPayloads, isEmpty);

      final replacementSocket = _testSocket();
      session.debugAttachSocketForTest(replacementSocket);
      session.debugHandleConnected();

      expect(replacementSocket.sentPayloads, hasLength(1));
      expect(replacementSocket.sentPayloads.single.first, 'EVENT');

      session.debugHandleSocketMessageForTest(['OK', _event().id, true, '']);
      await expectLater(publish, completion(_event()));
      expect(replacementSocket.sentPayloads, hasLength(1));
    },
  );

  test(
    'replays a dispatched ACK-lost event after a background interval beyond its acknowledgement timeout',
    () async {
      final session = RelaySessionNotifier();
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => session)],
      );
      addTearDown(container.dispose);
      container.read(relaySessionProvider);
      final subscription = container.listen(relaySessionProvider, (_, _) {});
      addTearDown(subscription.close);
      await Future<void>.delayed(Duration.zero);

      final event = _event();
      final disconnectedSocket = _testSocket();
      session.debugAttachSocketForTest(disconnectedSocket);
      session.debugHandleDisconnected();
      final publish = session.publish(event);
      final firstSocket = _testSocket();
      session.debugAttachSocketForTest(firstSocket);
      session.debugHandleConnected();
      expect(firstSocket.sentPayloads, hasLength(1));

      session.debugPauseNow();
      await Future<void>.delayed(const Duration(seconds: 9));
      final replacementSocket = _testSocket();
      session.debugAttachSocketForTest(replacementSocket);
      session.debugHandleConnected();

      expect(replacementSocket.sentPayloads, hasLength(1));
      expect(
        (replacementSocket.sentPayloads.single[1]
            as Map<String, dynamic>)['id'],
        event.id,
      );
      session.debugHandleSocketMessageForTest(['OK', event.id, true, '']);
      await expectLater(publish, completion(_event()));
    },
  );

  test('same-tick resume then send dispatches one replacement event', () async {
    final sockets = <_ControlledRelaySocket>[];
    final keychain = nostr.Keys.generate();
    final session = RelaySessionNotifier(
      socketFactory: _controlledSocketFactory(sockets),
    );
    final container = _authenticatedContainer(session, keychain.nsec);
    addTearDown(container.dispose);
    await container.read(authProvider.future);
    final subscription = container.listen(relaySessionProvider, (_, _) {});
    addTearDown(subscription.close);
    await Future<void>.delayed(Duration.zero);
    sockets.single.connectSuccessfully();

    session.debugHandleDisconnected();
    session.onAppResumed();
    final event = _event();
    final publish = session.publish(event);
    expect(sockets, hasLength(2));
    expect(sockets.last.sentPayloads, isEmpty);

    sockets.last.connectSuccessfully();
    expect(sockets.last.sentPayloads, hasLength(1));
    expect((sockets.last.sentPayloads.single[1] as Map)['id'], event.id);
    session.debugHandleSocketMessageForTest(['OK', event.id, true, '']);
    await expectLater(publish, completion(event));
  });

  test('same-tick send then resume dispatches one replacement event', () async {
    final sockets = <_ControlledRelaySocket>[];
    final keychain = nostr.Keys.generate();
    final session = RelaySessionNotifier(
      socketFactory: _controlledSocketFactory(sockets),
    );
    final container = _authenticatedContainer(session, keychain.nsec);
    addTearDown(container.dispose);
    await container.read(authProvider.future);
    final subscription = container.listen(relaySessionProvider, (_, _) {});
    addTearDown(subscription.close);
    await Future<void>.delayed(Duration.zero);
    sockets.single.connectSuccessfully();

    session.debugHandleDisconnected();
    final event = _event();
    final publish = session.publish(event);
    session.onAppResumed();
    expect(sockets, hasLength(2));
    expect(sockets.last.sentPayloads, isEmpty);

    sockets.last.connectSuccessfully();
    expect(sockets.last.sentPayloads, hasLength(1));
    expect((sockets.last.sentPayloads.single[1] as Map)['id'], event.id);
    session.debugHandleSocketMessageForTest(['OK', event.id, true, '']);
    await expectLater(publish, completion(event));
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

  test(
    'reconnect cancels an in-flight history request so it can be retried',
    () async {
      final session = RelaySessionNotifier();
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => session)],
      );
      addTearDown(container.dispose);
      container.read(relaySessionProvider);
      final subscription = container.listen(relaySessionProvider, (_, _) {});
      addTearDown(subscription.close);
      final first = session.fetchHistory(const NostrFilter(kinds: [39002]));
      final firstExpectation = expectLater(first, throwsException);
      session.debugHandleDisconnected();
      await firstExpectation;

      final retry = session.fetchHistory(const NostrFilter(kinds: [39002]));
      session.debugHandleMessage(['EOSE', 'h-2']);
      await expectLater(retry, completion(isEmpty));
    },
  );

  test(
    'disconnect flushes mixed-age buffered events before reconnect',
    () async {
      final sockets = <_ControlledRelaySocket>[];
      final keychain = nostr.Keys.generate();
      final session = RelaySessionNotifier(
        socketFactory: _controlledSocketFactory(sockets),
      );
      final container = _authenticatedContainer(session, keychain.nsec);
      addTearDown(container.dispose);
      await container.read(authProvider.future);
      final subscription = container.listen(relaySessionProvider, (_, _) {});
      addTearDown(subscription.close);
      await Future<void>.delayed(Duration.zero);
      sockets.single.connectSuccessfully();
      final delivered = <String>[];
      const filter = NostrFilter(kinds: EventKind.channelEventKinds, limit: 50);
      final subscribe = session.subscribe(
        filter,
        (event) => delivered.add(event.id),
      );
      session.debugHandleMessage(['EOSE', 'l-1']);
      final unsubscribe = await subscribe;

      session.debugHandleMessage([
        'EVENT',
        'l-1',
        _eventWithId('old').toJson(),
      ]);
      session.debugHandleMessage([
        'EVENT',
        'l-1',
        _eventWithId('new').toJson(),
      ]);
      sockets.single.disconnectWith(null);

      expect(delivered, ['event-old', 'event-new']);
      unsubscribe();
    },
  );

  test(
    'retains recent delivery keys across a bounded dedup rollover',
    () async {
      RelaySessionNotifier.debugRecentDeliveryKeyLimit = 3;
      addTearDown(
        () => RelaySessionNotifier.debugRecentDeliveryKeyLimit = 5000,
      );
      final session = RelaySessionNotifier();
      final delivered = <String>[];
      const filter = NostrFilter(
        kinds: EventKind.channelEventKinds,
        tags: {
          '#h': [_channelId],
        },
        limit: 50,
      );

      final subscribe = session.subscribe(
        filter,
        (event) => delivered.add(event.id),
      );
      session.debugHandleMessage(['EOSE', 'l-1']);
      final unsubscribe = await subscribe;
      for (var index = 1; index <= 4; index++) {
        session.debugHandleMessage([
          'EVENT',
          'l-1',
          _eventWithId('$index').toJson(),
        ]);
        session.debugFlushEventBuffer();
      }

      // A replay of the newest retained key must remain deduplicated after the
      // oldest key was evicted; clearing the whole set would redeliver it.
      session.debugHandleMessage(['EVENT', 'l-1', _eventWithId('4').toJson()]);
      session.debugFlushEventBuffer();
      expect(delivered, ['event-1', 'event-2', 'event-3', 'event-4']);
      unsubscribe();
    },
  );

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
  final List<List<dynamic>> sentPayloads = [];
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

  @override
  bool send(List<dynamic> payload) {
    sentPayloads.add(payload);
    return true;
  }

  void connectSuccessfully() => _connected();

  void disconnectWith(Object? error) => _disconnected(error);
}

_ControlledRelaySocket _testSocket() => _ControlledRelaySocket(
  wsUrl: 'wss://relay.example',
  nsec: null,
  onMessage: (_) {},
  onConnected: () {},
  onDisconnected: (_) {},
);

RelaySocketFactory _controlledSocketFactory(
  List<_ControlledRelaySocket> sockets,
) =>
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
    };

ProviderContainer _authenticatedContainer(
  RelaySessionNotifier session,
  String nsec,
) => ProviderContainer(
  overrides: [
    relaySessionProvider.overrideWith(() => session),
    relayConfigProvider.overrideWith(
      () => _FakeRelayConfigNotifier(
        baseUrl: 'https://relay.example',
        nsec: nsec,
      ),
    ),
    authProvider.overrideWith(() => _AuthenticatedAuthNotifier()),
  ],
);

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

NostrEvent _eventWithId(String suffix) {
  return NostrEvent(
    id: 'event-$suffix',
    pubkey: 'alice',
    createdAt: 20,
    kind: EventKind.streamMessageV2,
    tags: const [
      ['h', _channelId],
    ],
    content: 'hello',
    sig: 'sig',
  );
}
