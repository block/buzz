import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  group('relayResumeSince', () {
    // (original since, until, last seen, live until) -> resume since.
    final cases = <(int?, int?, int?, int?, int?)>[
      (null, null, null, null, null), // nothing known: original filter
      (500, null, null, null, 500), // original since kept
      (null, null, 1000, null, 995), // last seen - 5s
      (null, null, null, 1000, 40), // live until - 960s
      (null, null, 1000, 1010, 995), // last seen newer than live window
      (null, null, 1000, 2100, 1140), // live window newer than last seen
      (2000, null, 1000, 2100, 2000), // original since is a floor
      (null, null, 3, null, 0), // clamped at zero
      (null, null, null, 30, 0),
      (null, 900, 1000, 2100, 900), // never past the filter's until
      // An event stored right after the close can be backdated 900s (the
      // relay's timestamp drift bound); the window must reach at least that
      // far behind the close.
      (null, null, null, 5000, 4040),
    ];
    test('the live window covers the relay timestamp drift bound', () {
      const driftBound = 900;
      expect(
        relayResumeSince(
          originalSince: null,
          until: null,
          lastSeenCreatedAt: null,
          liveUntil: 5000,
        )!,
        lessThanOrEqualTo(5000 - driftBound),
      );
    });

    for (final (original, until, lastSeen, liveUntil, expected) in cases) {
      test('since=$original until=$until seen=$lastSeen live=$liveUntil', () {
        expect(
          relayResumeSince(
            originalSince: original,
            until: until,
            lastSeenCreatedAt: lastSeen,
            liveUntil: liveUntil,
          ),
          expected,
        );
      });
    }
  });

  group('relayReconnectDelay', () {
    // (backoff, close code, jitter unit, spread unit) -> delay in ms.
    final cases = <(int, int?, double, double, int)>[
      (1000, null, 0.0, 0.9, 750), // symmetric jitter, low edge
      (1000, null, 0.5, 0.9, 1000),
      (1000, null, 0.999, 0.9, 1250), // high edge
      (30000, null, 0.999, 0.9, 30000), // capped
      (1000, 1006, 0.5, 0.9, 1000), // abnormal close is not a restart
      (1000, 1012, 0.5, 0.0, 0), // restart: first attempt uniform [0, 10s]
      (1000, 1012, 0.5, 0.35, 3500),
      (1000, 1001, 0.5, 0.999, 9990),
      (4000, 1012, 0.5, 0.1, 4000), // already backing off: not below backoff
      (2000, 1012, 0.5, 0.9, 9000), // ... but still spread over the window
    ];
    for (final (backoff, code, jitter, spread, expected) in cases) {
      test('backoff=$backoff code=$code jitter=$jitter spread=$spread', () {
        expect(
          relayReconnectDelay(
            backoffMs: backoff,
            closeCode: code,
            jitterUnit: jitter,
            spreadUnit: spread,
          ).inMilliseconds,
          expected,
        );
      });
    }

    test('a connection is stable from 60 seconds', () {
      expect(isRelayConnectionStable(const Duration(seconds: 59)), isFalse);
      expect(isRelayConnectionStable(const Duration(seconds: 60)), isTrue);
    });
  });

  test('RelaySocket surfaces the relay close code', () async {
    // dart:io's server refuses to send 1012, so write the close frame by hand.
    final server = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
    server.listen((client) {
      client.listen((data) {
        final key = RegExp(
          r'Sec-WebSocket-Key: (.*)\r\n',
          caseSensitive: false,
        ).firstMatch(String.fromCharCodes(data));
        if (key == null) return;
        final accept = base64.encode(
          sha1
              .convert(
                utf8.encode(
                  '${key.group(1)!.trim()}258EAFA5-E914-47DA-95CA-C5AB0DC85B11',
                ),
              )
              .bytes,
        );
        client.write(
          'HTTP/1.1 101 Switching Protocols\r\n'
          'Upgrade: websocket\r\nConnection: Upgrade\r\n'
          'Sec-WebSocket-Accept: $accept\r\n\r\n',
        );
        client.add([0x88, 2, 1012 >> 8, 1012 & 0xff]);
      }, onError: (_) {});
    });
    final closeCode = Completer<int?>();
    late final RelaySocket socket;
    socket = RelaySocket(
      wsUrl: 'ws://127.0.0.1:${server.port}',
      nsec: null,
      onMessage: (_) {},
      onConnected: () {},
      onDisconnected: (_) {
        if (!closeCode.isCompleted) closeCode.complete(socket.closeCode);
      },
    );
    unawaited(socket.connect());
    expect(await closeCode.future.timeout(const Duration(seconds: 5)), 1012);
    socket.dispose();
    await server.close();
  });

  group('reconnect through RelaySessionNotifier', () {
    test('a bounced client keeps backing off until a connection is stable, '
        'and a restart close spreads the first attempt', () async {
      final h = await _Harness.start(
        random: _ScriptedRandom([
          0.5, 0.3, // 1012: first attempt, uniform 3s
          0.5, 0.1, // 1012 again within 60s: backoff 2s beats spread 1s
          0.0, 0.0, // plain close after a stable connection: base - 25%
          0.5, 0.0, // plain close after a short connection: doubled backoff
        ]),
      );
      h.sockets.last.connectSuccessfully();
      h.now = h.now.add(const Duration(seconds: 5));
      h.sockets.last.disconnectWith(null, closeCode: 1012);
      expect(h.reconnectDelays.last, const Duration(seconds: 3));

      h.timers.last.fire();
      await h.settle();
      h.sockets.last.connectSuccessfully();
      h.now = h.now.add(const Duration(seconds: 5));
      h.sockets.last.disconnectWith(null, closeCode: 1012);
      expect(h.reconnectDelays.last, const Duration(seconds: 2));

      h.timers.last.fire();
      await h.settle();
      h.sockets.last.connectSuccessfully();
      h.now = h.now.add(const Duration(seconds: 60));
      h.sockets.last.disconnectWith(null);
      expect(h.reconnectDelays.last, const Duration(milliseconds: 750));

      h.timers.last.fire();
      await h.settle();
      h.sockets.last.connectSuccessfully();
      h.now = h.now.add(const Duration(seconds: 59));
      h.sockets.last.disconnectWith(null);
      // Backoff was doubled to 2s when the previous attempt fired and the
      // connection did not stay up long enough to reset it.
      expect(h.reconnectDelays.last, const Duration(seconds: 2));
    });
  });

  group('resume cursor through RelaySessionNotifier', () {
    test('replays from the last frame received before the loss, not the '
        'original filter or the moment the loss is noticed, and only for '
        'subscriptions that were live', () async {
      final h = await _Harness.start();
      final start = h.nowSeconds;
      h.sockets.last.connectSuccessfully();

      await h.subscribeLive(_filter('quiet', since: start - 3600));
      h.subscribeWithoutEose(_filter('no-eose'));
      final busy = await h.subscribeLive(_filter('busy'));
      final closed = await h.subscribeLive(_filter('closed'));
      h.receive(['CLOSED', closed, 'error: transient']);
      h.now = h.now.add(const Duration(seconds: 120));
      h.receive(['EVENT', busy, _eventJson('e1', start + 100, 'busy')]);

      // The device sleeps for 2000s; the loss is only noticed on wake.
      h.now = h.now.add(const Duration(seconds: 2000));
      h.sockets.last.disconnectWith(null);
      h.timers.last.fire();
      await h.settle();
      h.sockets.last.connectSuccessfully();
      await h.settle();

      // A backdated event stored just after the last frame (up to 900s behind
      // the relay clock) is still inside the window.
      expect(h.replayedSince()['quiet']!, lessThanOrEqualTo(start + 120 - 900));
      expect(h.replayedSince(), {
        'quiet': start - 840, // last frame (start + 120) - 960s
        'no-eose': null, // never live: today's behaviour
        'busy': start + 95, // last seen - 5s beats last frame - 960s
        'closed': null, // relay closed it: not live at the disconnect
      });
    });

    test(
      'a background disconnect resumes from the last frame before it',
      () async {
        final h = await _Harness.start();
        final start = h.nowSeconds;
        h.sockets.last.connectSuccessfully();
        await h.subscribeLive(_filter('quiet'));

        h.session.onAppPaused();
        h.now = h.now.add(const Duration(seconds: 20));
        h.receive(['NOTICE', 'still here']);
        h.now = h.now.add(const Duration(seconds: 10));
        h.session.debugPauseNow();
        h.now = h.now.add(const Duration(minutes: 10));
        h.session.onAppResumed();
        await h.settle();
        h.sockets.last.connectSuccessfully();
        await h.settle();

        expect(h.replayedSince(), {'quiet': start + 20 - 960});
      },
    );

    test('a later connection that never reaches EOSE keeps the older cursor, '
        'then advances once live again', () async {
      final h = await _Harness.start();
      final start = h.nowSeconds;
      h.sockets.last.connectSuccessfully();
      await h.subscribeLive(_filter('quiet'));

      await h.dropAfter(const Duration(seconds: 100));
      expect(h.replayedSince(), {'quiet': start + 100 - 960});

      // The second connection dies before EOSE: the cursor must not move.
      await h.dropAfter(const Duration(seconds: 100));
      expect(h.replayedSince(), {'quiet': start + 100 - 960});

      h.receive(['EOSE', 'l-1']);
      await h.dropAfter(const Duration(seconds: 100));
      expect(h.replayedSince(), {'quiet': start + 300 - 960});
    });

    // A close or error that was buffered while the device slept is processed
    // on wake. It is the transport talking, not a frame from the relay, so it
    // must not count as the last time the relay was heard from.
    final losses = <(String, int?, Object?)>[
      ('close 1012', 1012, null),
      ('close 1006', 1006, null),
      ('socket error', null, Exception('connection reset')),
    ];
    for (final (label, closeCode, error) in losses) {
      test(
        'a $label noticed after an hour asleep does not extend liveness',
        () async {
          final h = await _Harness.start();
          final start = h.nowSeconds;
          h.sockets.last.connectSuccessfully();
          await h.subscribeLive(_filter('quiet'));

          h.now = h.now.add(const Duration(hours: 1));
          h.sockets.last.disconnectWith(error, closeCode: closeCode);
          h.timers.last.fire();
          await h.settle();
          h.sockets.last.connectSuccessfully();
          await h.settle();

          expect(h.replayedSince(), {'quiet': start - 960});
        },
      );
    }

    test('a delayed EOSE for an unsubscribed id cannot mark its replacement '
        'live', () async {
      final h = await _Harness.start();
      final start = h.nowSeconds;
      h.sockets.last.connectSuccessfully();
      final old = await h.subscribeLive(_filter('quiet', since: start - 3600));
      h.unsubscribers[old]!();

      // The channel is subscribed again with an older floor. The old REQ's
      // EOSE was still in flight when its CLOSE went out and now arrives.
      final replacement = h.subscribeWithoutEose(
        _filter('quiet', since: start - 7200),
      );
      expect(replacement, isNot(old));
      h.receive(['EOSE', old]);

      // The socket drops before the replacement's history arrives.
      await h.dropAfter(const Duration(seconds: 100));
      expect(h.replayedSince(), {'quiet': start - 7200});
    });

    test('a second REQ on one socket cannot advance the cursor: its EOSE may '
        'belong to the REQ it replaced', () async {
      final h = await _Harness.start();
      final start = h.nowSeconds;
      h.sockets.last.connectSuccessfully();
      await h.subscribeLive(_filter('quiet'));
      await h.dropAfter(const Duration(seconds: 100));
      expect(h.replayedSince(), {'quiet': start + 100 - 960});

      // On the new socket: EOSE, then CLOSED, then a same-socket retry REQ
      // whose EOSE arrives. The retry's history has not been confirmed.
      h.receive(['EOSE', 'l-1']);
      h.receive(['CLOSED', 'l-1', 'error: transient']);
      h.retryTimers.last.fire();
      await h.settle();
      expect(h.replayedSince(), {'quiet': start + 100 - 960});
      h.receive(['EOSE', 'l-1']);

      await h.dropAfter(const Duration(seconds: 500));
      expect(h.replayedSince(), {'quiet': start + 100 - 960});
    });
  });
}

NostrFilter _filter(String name, {int? since}) => NostrFilter(
  kinds: const [9],
  tags: {
    '#h': [name],
  },
  limit: 0,
  since: since,
);

Map<String, dynamic> _eventJson(String id, int createdAt, String channel) => {
  'id': id,
  'pubkey': 'alice',
  'created_at': createdAt,
  'kind': 9,
  'tags': [
    ['h', channel],
  ],
  'content': 'hello',
  'sig': 'sig',
};

class _Harness {
  _Harness._(this.session, this.now);

  final RelaySessionNotifier session;
  DateTime now;
  final sockets = <_FakeSocket>[];
  final timers = <_ManualTimer>[];
  final retryTimers = <_ManualTimer>[];
  final reconnectDelays = <Duration>[];
  final subscriptionNames = <String, String>{};
  final unsubscribers = <String, void Function()>{};

  int get nowSeconds => now.millisecondsSinceEpoch ~/ 1000;

  static Future<_Harness> start({Random? random}) async {
    final keys = nostr.Keys.generate();
    late final _Harness h;
    final session = RelaySessionNotifier(
      now: () => h.now,
      random: random,
      retryTimerFactory: (duration, callback) {
        final timer = _ManualTimer(callback);
        h.retryTimers.add(timer);
        return timer;
      },
      reconnectTimerFactory: (duration, callback) {
        final timer = _ManualTimer(callback);
        h.timers.add(timer);
        h.reconnectDelays.add(duration);
        return timer;
      },
      socketFactory:
          ({
            required wsUrl,
            required nsec,
            required onMessage,
            required onConnected,
            required onDisconnected,
          }) {
            final socket = _FakeSocket(
              wsUrl: wsUrl,
              nsec: nsec,
              onMessage: onMessage,
              onConnected: onConnected,
              onDisconnected: onDisconnected,
            );
            h.sockets.add(socket);
            return socket;
          },
    );
    h = _Harness._(session, DateTime.utc(2026, 10, 28, 12));
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => session),
        relayConfigProvider.overrideWith(
          () => _FakeRelayConfigNotifier(nsec: keys.nsec),
        ),
        authProvider.overrideWith(() => _AuthenticatedAuthNotifier()),
      ],
    );
    addTearDown(container.dispose);
    await container.read(authProvider.future);
    final listener = container.listen(relaySessionProvider, (_, _) {});
    addTearDown(listener.close);
    await h.settle();
    expect(h.sockets, hasLength(1));
    return h;
  }

  Future<void> settle() => Future<void>.delayed(Duration.zero);

  /// Delivers [frame] from the relay over the latest socket.
  void receive(List<dynamic> frame) => sockets.last.receive(frame);

  /// Lets [elapsed] pass with a last frame at its end, then drops the socket
  /// and reconnects, as a reconnect timer firing and AUTH completing would.
  Future<void> dropAfter(Duration elapsed) async {
    now = now.add(elapsed);
    receive(['NOTICE', 'ping']);
    sockets.last.disconnectWith(null);
    timers.last.fire();
    await settle();
    sockets.last.connectSuccessfully();
    await settle();
  }

  /// Subscribes and delivers EOSE, returning the subscription id.
  Future<String> subscribeLive(NostrFilter filter) async {
    final subId = 'l-${subscriptionNames.length + 1}';
    subscriptionNames[subId] = filter.tags['#h']!.single;
    final future = session.subscribe(filter, (_) {});
    receive(['EOSE', subId]);
    unsubscribers[subId] = await future;
    return subId;
  }

  /// Subscribes without EOSE, returning the subscription id.
  String subscribeWithoutEose(NostrFilter filter) {
    final subId = 'l-${subscriptionNames.length + 1}';
    subscriptionNames[subId] = filter.tags['#h']!.single;
    unawaited(session.subscribe(filter, (_) {}));
    return subId;
  }

  /// The `since` of each subscription's REQ on the latest socket, by channel.
  Map<String, int?> replayedSince() => {
    for (final message in sockets.last.sent.where((m) => m.first == 'REQ'))
      subscriptionNames[message[1] as String]!:
          (message[2] as Map<String, dynamic>)['since'] as int?,
  };
}

class _FakeSocket extends RelaySocket {
  _FakeSocket({
    required super.wsUrl,
    required super.nsec,
    required super.onMessage,
    required super.onConnected,
    required super.onDisconnected,
  }) : _connected = onConnected,
       _disconnected = onDisconnected,
       _message = onMessage;

  final void Function() _connected;
  final void Function(Object? error) _disconnected;
  final void Function(List<dynamic> message) _message;
  final sent = <List<dynamic>>[];
  int? _fakeCloseCode;

  @override
  int? get closeCode => _fakeCloseCode;

  @override
  Future<void> connect() async {}

  @override
  void send(List<dynamic> payload) => sent.add(payload);

  @override
  void dispose() {}

  void connectSuccessfully() => _connected();

  void receive(List<dynamic> frame) => _message(frame);

  void disconnectWith(Object? error, {int? closeCode}) {
    _fakeCloseCode = closeCode;
    _disconnected(error);
  }
}

class _ManualTimer implements Timer {
  _ManualTimer(this._callback);

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

/// A [Random] whose doubles come from a script.
class _ScriptedRandom implements Random {
  _ScriptedRandom(List<double> doubles) : _doubles = List.of(doubles);

  final List<double> _doubles;

  @override
  double nextDouble() => _doubles.removeAt(0);

  @override
  bool nextBool() => throw UnimplementedError();

  @override
  int nextInt(int max) => throw UnimplementedError();
}

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  _FakeRelayConfigNotifier({required String? nsec}) : _nsec = nsec;

  final String? _nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: _nsec);
}

class _AuthenticatedAuthNotifier extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.authenticated);
}
