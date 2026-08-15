import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/features/pairing/pairing_socket.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

const _privateKey =
    '09b3065e3570a3a4054660dccd66e12774a99a904fdb0ca02dbc6c3136249506';

void main() {
  group('PairingSocket', () {
    test('connects when the pairing relay sends no AUTH challenge', () async {
      final server = await _TestRelay.start((_) {});
      addTearDown(server.close);
      final socket = _socket(
        server.url,
        authChallengeTimeout: const Duration(milliseconds: 30),
      );
      addTearDown(socket.disconnect);

      await socket.connect();

      expect(socket.isConnected, isTrue);
    });

    test('reports the open-relay connection stages in order', () async {
      final server = await _TestRelay.start((_) {});
      addTearDown(server.close);
      final stages = <PairingStage>[];
      final socket = _socket(
        server.url,
        authChallengeTimeout: const Duration(milliseconds: 30),
        onStage: stages.add,
      );
      addTearDown(socket.disconnect);

      await socket.connect();

      expect(stages, [
        PairingStage.openingWebSocket,
        PairingStage.webSocketOpen,
        PairingStage.waitingForAuth,
        PairingStage.connected,
      ]);
    });

    test('fails closed when the WebSocket open deadline expires', () async {
      final channel = _NeverReadyWebSocketChannel();
      final stages = <PairingStage>[];
      var disconnectCount = 0;
      final socket = _socket(
        'wss://pairing.example.test/pair',
        connectionTimeout: const Duration(milliseconds: 20),
        channelFactory: (_) => channel,
        onDisconnected: (_) => disconnectCount++,
        onStage: stages.add,
      );

      await expectLater(
        socket.connect(),
        throwsA(
          isA<PairingConnectionException>().having(
            (error) => error.stage,
            'stage',
            PairingStage.openingWebSocket,
          ),
        ),
      );

      expect(stages, [PairingStage.openingWebSocket]);
      expect(disconnectCount, 1);
      expect(channel.closed, isTrue);
    });

    test('reports timeout even when pre-ready close never completes', () async {
      final channel = _NeverReadyWebSocketChannel(closeCompletes: false);
      final socket = _socket(
        'wss://pairing.example.test/pair',
        connectionTimeout: const Duration(milliseconds: 20),
        channelFactory: (_) => channel,
      );

      await expectLater(
        socket.connect().timeout(const Duration(seconds: 2)),
        throwsA(isA<PairingConnectionException>()),
      );
    });

    test('dispose settles a pending authentication wait', () async {
      final channel = _ControlledWebSocketChannel();
      final socket = _socket(
        'wss://pairing.example.test/pair',
        authChallengeTimeout: const Duration(seconds: 30),
        channelFactory: (_) => channel,
      );

      final connect = socket.connect();
      final expectation = expectLater(
        connect.timeout(const Duration(seconds: 1)),
        throwsA(isA<PairingCanceledException>()),
      );
      await Future<void>.delayed(Duration.zero);
      socket.dispose();

      await expectation;
    });

    test('answers an AUTH challenge and requires an accepted OK', () async {
      final authReceived = Completer<List<dynamic>>();
      final stages = <PairingStage>[];
      final server = await _TestRelay.start((webSocket) async {
        webSocket.add(jsonEncode(['AUTH', 'challenge']));
        final auth =
            jsonDecode(await webSocket.first as String) as List<dynamic>;
        authReceived.complete(auth);
        final event = auth[1] as Map<String, dynamic>;
        webSocket.add(jsonEncode(['OK', event['id'], true, 'authenticated']));
      });
      addTearDown(server.close);
      final socket = _socket(server.url, onStage: stages.add);
      addTearDown(socket.disconnect);

      await socket.connect();

      expect(socket.isConnected, isTrue);
      expect((await authReceived.future).first, 'AUTH');
      expect(stages, [
        PairingStage.openingWebSocket,
        PairingStage.webSocketOpen,
        PairingStage.waitingForAuth,
        PairingStage.authenticating,
        PairingStage.connected,
      ]);
    });

    test('fails when the pairing relay rejects AUTH', () async {
      final server = await _TestRelay.start((webSocket) async {
        webSocket.add(jsonEncode(['AUTH', 'challenge']));
        final auth =
            jsonDecode(await webSocket.first as String) as List<dynamic>;
        final event = auth[1] as Map<String, dynamic>;
        webSocket.add(jsonEncode(['OK', event['id'], false, 'bad auth']));
      });
      addTearDown(server.close);
      var disconnectCount = 0;
      final socket = _socket(
        server.url,
        onDisconnected: (_) => disconnectCount++,
      );
      addTearDown(socket.disconnect);

      await expectLater(socket.connect(), throwsA(isA<PairingAuthException>()));

      expect(socket.isConnected, isFalse);
      expect(disconnectCount, 1);
    });

    test(
      'answers a challenge after the optional AUTH wait completes',
      () async {
        final authReceived = Completer<void>();
        final server = await _TestRelay.start((webSocket) async {
          await Future<void>.delayed(const Duration(milliseconds: 80));
          webSocket.add(jsonEncode(['AUTH', 'late-challenge']));
          final auth =
              jsonDecode(await webSocket.first as String) as List<dynamic>;
          final event = auth[1] as Map<String, dynamic>;
          webSocket.add(jsonEncode(['OK', event['id'], true, 'authenticated']));
          authReceived.complete();
        });
        addTearDown(server.close);
        final socket = _socket(
          server.url,
          authChallengeTimeout: const Duration(milliseconds: 30),
        );
        addTearDown(socket.disconnect);

        await socket.connect();
        await authReceived.future;

        expect(socket.isConnected, isTrue);
      },
    );

    test('fails when AUTH receives no OK response', () async {
      final server = await _TestRelay.start((webSocket) {
        webSocket.add(jsonEncode(['AUTH', 'challenge']));
      });
      addTearDown(server.close);
      final socket = _socket(
        server.url,
        authResponseTimeout: const Duration(milliseconds: 100),
      );
      addTearDown(socket.disconnect);

      await expectLater(socket.connect(), throwsA(isA<PairingAuthException>()));

      expect(socket.isConnected, isFalse);
    });

    test('notifies once when a connected stream emits an error', () async {
      var disconnectCount = 0;
      final channel = _ControlledWebSocketChannel();
      final socket = _socket(
        'ws://unused',
        onDisconnected: (_) => disconnectCount++,
        channelFactory: (_) => channel,
        authChallengeTimeout: Duration.zero,
      );
      addTearDown(socket.disconnect);

      await socket.connect();
      channel.emitError(Exception('stream failed'));
      await Future<void>.delayed(Duration.zero);

      expect(disconnectCount, 1);
    });

    test('notifies once when a connected stream closes', () async {
      var disconnectCount = 0;
      final channel = _ControlledWebSocketChannel();
      final socket = _socket(
        'ws://unused',
        onDisconnected: (_) => disconnectCount++,
        channelFactory: (_) => channel,
        authChallengeTimeout: Duration.zero,
      );
      addTearDown(socket.disconnect);

      await socket.connect();
      await channel.closeStream();

      expect(disconnectCount, 1);
    });

    test(
      'does not notify when deliberately disconnected or disposed',
      () async {
        var disconnectCount = 0;
        final disconnectChannel = _ControlledWebSocketChannel();
        final disconnectingSocket = _socket(
          'ws://unused',
          onDisconnected: (_) => disconnectCount++,
          channelFactory: (_) => disconnectChannel,
          authChallengeTimeout: Duration.zero,
        );
        await disconnectingSocket.connect();

        await disconnectingSocket.disconnect();

        final disposeChannel = _ControlledWebSocketChannel();
        final disposingSocket = _socket(
          'ws://unused',
          onDisconnected: (_) => disconnectCount++,
          channelFactory: (_) => disposeChannel,
          authChallengeTimeout: Duration.zero,
        );
        await disposingSocket.connect();

        disposingSocket.dispose();
        await Future<void>.delayed(Duration.zero);

        expect(disconnectCount, 0);
      },
    );
  });
}

PairingSocket _socket(
  String url, {
  Duration connectionTimeout = const Duration(seconds: 10),
  Duration authChallengeTimeout = const Duration(milliseconds: 500),
  Duration authResponseTimeout = const Duration(seconds: 10),
  void Function(Object? error)? onDisconnected,
  void Function(PairingStage stage)? onStage,
  WebSocketChannel Function(Uri uri)? channelFactory,
}) => PairingSocket(
  wsUrl: url,
  ephemeralPrivkey: _privateKey,
  onMessage: (_) {},
  onDisconnected: onDisconnected ?? (_) {},
  onStage: onStage,
  connectionTimeout: connectionTimeout,
  authChallengeTimeout: authChallengeTimeout,
  authResponseTimeout: authResponseTimeout,
  channelFactory: channelFactory ?? WebSocketChannel.connect,
);

class _NeverReadyWebSocketChannel implements WebSocketChannel {
  _NeverReadyWebSocketChannel({bool closeCompletes = true})
    : _sink = _TrackingWebSocketSink(closeCompletes: closeCompletes);

  final Completer<void> _ready = Completer<void>();
  final StreamController<dynamic> _streamController = StreamController();
  final _TrackingWebSocketSink _sink;

  bool get closed => _sink.closed;

  @override
  Future<void> get ready => _ready.future;

  @override
  Stream<dynamic> get stream => _streamController.stream;

  @override
  WebSocketSink get sink => _sink;

  @override
  int? get closeCode => null;

  @override
  String? get closeReason => null;

  @override
  String? get protocol => null;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _TrackingWebSocketSink extends _ControlledWebSocketSink {
  _TrackingWebSocketSink({required this.closeCompletes});

  final bool closeCompletes;
  bool closed = false;

  @override
  Future<void> close([int? closeCode, String? closeReason]) async {
    closed = true;
    if (!closeCompletes) {
      await Completer<void>().future;
    }
  }
}

class _ControlledWebSocketChannel implements WebSocketChannel {
  final StreamController<dynamic> _streamController = StreamController();
  final WebSocketSink _sink = _ControlledWebSocketSink();

  void emitError(Object error) => _streamController.addError(error);

  Future<void> closeStream() => _streamController.close();

  @override
  Future<void> get ready => Future.value();

  @override
  Stream<dynamic> get stream => _streamController.stream;

  @override
  WebSocketSink get sink => _sink;

  @override
  int? get closeCode => null;

  @override
  String? get closeReason => null;

  @override
  String? get protocol => null;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _ControlledWebSocketSink implements WebSocketSink {
  @override
  void add(dynamic event) {}

  @override
  void addError(Object error, [StackTrace? stackTrace]) {}

  @override
  Future<void> addStream(Stream<dynamic> stream) async {
    await stream.drain<void>();
  }

  @override
  Future<void> close([int? closeCode, String? closeReason]) async {}

  @override
  Future<void> get done => Future.value();
}

class _TestRelay {
  final HttpServer _server;
  final List<WebSocket> _sockets = [];

  _TestRelay._(this._server);

  String get url => 'ws://${_server.address.host}:${_server.port}';

  static Future<_TestRelay> start(
    FutureOr<void> Function(WebSocket socket) onConnected,
  ) async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final relay = _TestRelay._(server);
    server.listen((request) async {
      final socket = await WebSocketTransformer.upgrade(request);
      relay._sockets.add(socket);
      await onConnected(socket);
    });
    return relay;
  }

  Future<void> close() async {
    for (final socket in _sockets) {
      await socket.close();
    }
    await _server.close(force: true);
  }
}
