import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/features/pairing/pairing_socket.dart';
import 'package:flutter_test/flutter_test.dart';

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
      final socket = _socket(server.url);
      addTearDown(socket.disconnect);

      await expectLater(socket.connect(), throwsA(isA<PairingAuthException>()));

      expect(socket.isConnected, isFalse);
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
  });
}

PairingSocket _socket(
  String url, {
  Duration authChallengeTimeout = const Duration(milliseconds: 500),
  Duration authResponseTimeout = const Duration(seconds: 10),
}) => PairingSocket(
  wsUrl: url,
  ephemeralPrivkey: _privateKey,
  onMessage: (_) {},
  onDisconnected: (_) {},
  authChallengeTimeout: authChallengeTimeout,
  authResponseTimeout: authResponseTimeout,
);

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
