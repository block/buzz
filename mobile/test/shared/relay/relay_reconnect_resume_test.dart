import 'dart:async';
import 'dart:io';
import 'dart:math';

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
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
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    server.listen((request) async {
      final ws = await WebSocketTransformer.upgrade(request);
      await ws.close(1012, 'service restart');
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
    await server.close(force: true);
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
    test('replays from the end of live delivery, not the original filter, '
        'only for subscriptions that were live', () async {
      final h = await _Harness.start();
      final start = h.nowSeconds;
      h.sockets.last.connectSuccessfully();

      await h.subscribeLive(_filter('quiet', since: start - 3600));
      h.subscribeWithoutEose(_filter('no-eose'));
      final busy = await h.subscribeLive(_filter('busy'));
      final closed = await h.subscribeLive(_filter('closed'));
      h.session.debugHandleMessage(['CLOSED', closed, 'error: transient']);
      h.session.debugHandleMessage([
        'EVENT',
        busy,
        _eventJson('e1', start + 100, 'busy'),
      ]);

      h.now = h.now.add(const Duration(seconds: 120));
      h.sockets.last.disconnectWith(null);
      h.timers.last.fire();
      await h.settle();
      h.sockets.last.connectSuccessfully();
      await h.settle();

      // A backdated event stored just after the close (up to 900s behind the
      // relay clock) is still inside the window.
      expect(h.replayedSince()['quiet']!, lessThanOrEqualTo(start + 120 - 900));
      expect(h.replayedSince(), {
        'quiet': start - 840, // live until (start + 120) - 960s
        'no-eose': null, // never live: today's behaviour
        'busy': start + 95, // last seen - 5s beats live until - 960s
        'closed': null, // relay closed it: not live at the disconnect
      });
    });

    test('a background disconnect ends live delivery at pause time', () async {
      final h = await _Harness.start();
      final start = h.nowSeconds;
      h.sockets.last.connectSuccessfully();
      await h.subscribeLive(_filter('quiet'));

      h.session.onAppPaused();
      h.now = h.now.add(const Duration(seconds: 30));
      h.session.debugPauseNow();
      h.now = h.now.add(const Duration(minutes: 10));
      h.session.onAppResumed();
      await h.settle();
      h.sockets.last.connectSuccessfully();
      await h.settle();

      expect(h.replayedSince(), {'quiet': start + 30 - 960});
    });

    test('a later connection that never reaches EOSE keeps the older cursor, '
        'then advances once live again', () async {
      final h = await _Harness.start();
      final start = h.nowSeconds;
      h.sockets.last.connectSuccessfully();
      await h.subscribeLive(_filter('quiet'));

      h.now = h.now.add(const Duration(seconds: 100));
      h.sockets.last.disconnectWith(null);
      h.timers.last.fire();
      await h.settle();
      h.sockets.last.connectSuccessfully();
      await h.settle();
      expect(h.replayedSince(), {'quiet': start + 100 - 960});

      // The second connection dies before EOSE: the cursor must not move.
      h.now = h.now.add(const Duration(seconds: 100));
      h.sockets.last.disconnectWith(null);
      h.timers.last.fire();
      await h.settle();
      h.sockets.last.connectSuccessfully();
      await h.settle();
      expect(h.replayedSince(), {'quiet': start + 100 - 960});

      h.session.debugHandleMessage(['EOSE', 'l-1']);
      h.now = h.now.add(const Duration(seconds: 100));
      h.sockets.last.disconnectWith(null);
      h.timers.last.fire();
      await h.settle();
      h.sockets.last.connectSuccessfully();
      await h.settle();
      expect(h.replayedSince(), {'quiet': start + 300 - 960});
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
  final reconnectDelays = <Duration>[];
  final subscriptionNames = <String, String>{};

  int get nowSeconds => now.millisecondsSinceEpoch ~/ 1000;

  static Future<_Harness> start({Random? random}) async {
    final keys = nostr.Keys.generate();
    late final _Harness h;
    final session = RelaySessionNotifier(
      now: () => h.now,
      random: random,
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

  /// Subscribes and delivers EOSE, returning the subscription id.
  Future<String> subscribeLive(NostrFilter filter) async {
    final subId = 'l-${subscriptionNames.length + 1}';
    subscriptionNames[subId] = filter.tags['#h']!.single;
    final future = session.subscribe(filter, (_) {});
    session.debugHandleMessage(['EOSE', subId]);
    await future;
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
       _disconnected = onDisconnected;

  final void Function() _connected;
  final void Function(Object? error) _disconnected;
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
