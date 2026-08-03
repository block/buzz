import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/relay/relay_socket.dart';
import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';

Future<ServerSocket> _silentAfterHandshakeServer() async {
  final server = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
  server.listen((client) {
    client.listen(
      (data) {
        final match = RegExp(
          r'Sec-WebSocket-Key: (.*)\r\n',
          caseSensitive: false,
        ).firstMatch(String.fromCharCodes(data));
        if (match == null) return;
        final accept = base64.encode(
          sha1
              .convert(
                utf8.encode(
                  '${match.group(1)!.trim()}258EAFA5-E914-47DA-95CA-C5AB0DC85B11',
                ),
              )
              .bytes,
        );
        client.write(
          'HTTP/1.1 101 Switching Protocols\r\n'
          'Upgrade: websocket\r\n'
          'Connection: Upgrade\r\n'
          'Sec-WebSocket-Accept: $accept\r\n\r\n',
        );
      },
      onError: (_) {},
      onDone: () {},
    );
  });
  return server;
}

void main() {
  const testPingInterval = Duration(milliseconds: 150);

  setUp(() {
    RelaySocket.debugPingInterval = testPingInterval;
  });

  tearDown(() {
    RelaySocket.debugPingInterval = RelaySocket.pingInterval;
  });

  test('detects a peer that stops answering pings', () async {
    final server = await _silentAfterHandshakeServer();
    final disconnected = Completer<Object?>();
    final socket = RelaySocket(
      wsUrl: 'ws://127.0.0.1:${server.port}',
      nsec: null,
      onMessage: (_) {},
      onConnected: () {},
      onDisconnected: (error) {
        if (!disconnected.isCompleted) disconnected.complete(error);
      },
    );
    unawaited(socket.connect());

    await expectLater(
      disconnected.future.timeout(testPingInterval * 4),
      completes,
    );

    socket.dispose();
    await server.close();
  });

  test('keeps an idle healthy peer connected', () async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    server.transform(WebSocketTransformer()).listen((socket) {
      socket.listen((_) {}, onError: (_) {}, onDone: () {});
    });
    var disconnected = false;
    final socket = RelaySocket(
      wsUrl: 'ws://127.0.0.1:${server.port}',
      nsec: null,
      onMessage: (_) {},
      onConnected: () {},
      onDisconnected: (_) => disconnected = true,
    );
    unawaited(socket.connect());

    await Future<void>.delayed(testPingInterval * 4);
    expect(disconnected, isFalse);

    await socket.disconnect();
    await server.close(force: true);
  });
}
