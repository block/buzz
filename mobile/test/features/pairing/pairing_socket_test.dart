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

    test('answers an AUTH challenge and requires an accepted OK', () async {
      final authReceived = Completer<List<dynamic>>();
      final server = await _TestRelay.start((webSocket) async {
        webSocket.add(jsonEncode(['AUTH', 'challenge']));
        final auth =
            jsonDecode(await webSocket.first as String) as List<dynamic>;
        authReceived.complete(auth);
        final event = auth[1] as Map<String, dynamic>;
        webSocket.add(jsonEncode(['OK', event['id'], true, 'authenticated']));
      });
      addTearDown(server.close);
      final socket = _socket(server.url);
      addTearDown(socket.disconnect);

      await socket.connect();

      expect(socket.isConnected, isTrue);
      expect((await authReceived.future).first, 'AUTH');
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
  Duration authChallengeTimeout = const Duration(milliseconds: 500),
  Duration authResponseTimeout = const Duration(seconds: 10),
  void Function(Object? error)? onDisconnected,
  WebSocketChannel Function(Uri uri)? channelFactory,
}) => PairingSocket(
  wsUrl: url,
  ephemeralPrivkey: _privateKey,
  onMessage: (_) {},
  onDisconnected: onDisconnected ?? (_) {},
  authChallengeTimeout: authChallengeTimeout,
  authResponseTimeout: authResponseTimeout,
  channelFactory: channelFactory ?? WebSocketChannel.connect,
);

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
