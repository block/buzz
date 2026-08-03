import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:buzz/shared/relay/relay_socket.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pointycastle/digests/sha1.dart';

String? _readWebSocketKey(BytesBuilder requestBytes, List<int> chunk) {
  requestBytes.add(chunk);
  final request = String.fromCharCodes(requestBytes.toBytes());
  if (!request.contains('\r\n\r\n')) return null;

  return RegExp(
    r'Sec-WebSocket-Key: (.*)\r\n',
    caseSensitive: false,
  ).firstMatch(request)?.group(1)?.trim();
}

/// A server that completes the WS handshake then never speaks again: no pongs,
/// no close frame. Only a client-side ping timeout can notice.
Future<ServerSocket> _silentAfterHandshakeServer() async {
  final server = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
  server.listen((client) {
    final requestBytes = BytesBuilder(copy: false);
    var handshakeCompleted = false;
    client.listen(
      (data) {
        if (handshakeCompleted) return;
        final key = _readWebSocketKey(requestBytes, data);
        if (key == null) return;
        handshakeCompleted = true;
        final accept = base64.encode(
          SHA1Digest().process(
            Uint8List.fromList(
              utf8.encode('${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11'),
            ),
          ),
        );
        client.add(
          utf8.encode(
            'HTTP/1.1 101 Switching Protocols\r\n'
            'Upgrade: websocket\r\nConnection: Upgrade\r\n'
            'Sec-WebSocket-Accept: $accept\r\n\r\n',
          ),
        );
      },
      onError: (_) {},
      onDone: () {},
    );
  });
  return server;
}

Future<void> _expectWebSocketHandshakeCompleted(RelaySocket socket) async {
  final timeout = Stopwatch()..start();
  while (socket.state == SocketState.connecting &&
      timeout.elapsed < const Duration(seconds: 1)) {
    await Future<void>.delayed(const Duration(milliseconds: 1));
  }

  expect(
    socket.state,
    SocketState.authenticating,
    reason: 'The WebSocket handshake must complete before ping detection',
  );
}

void main() {
  const testPingInterval = Duration(milliseconds: 150);

  setUp(() {
    RelaySocket.debugPingInterval = testPingInterval;
  });

  tearDown(() {
    RelaySocket.debugPingInterval = RelaySocket.pingInterval;
  });

  test('buffers fragmented WebSocket request headers', () {
    final requestBytes = BytesBuilder(copy: false);

    expect(
      _readWebSocketKey(
        requestBytes,
        utf8.encode('GET / HTTP/1.1\r\nSec-WebSocket-K'),
      ),
      isNull,
    );
    expect(
      _readWebSocketKey(
        requestBytes,
        utf8.encode('ey: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n'),
      ),
      'dGhlIHNhbXBsZSBub25jZQ==',
    );
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

    await _expectWebSocketHandshakeCompleted(socket);

    var detected = true;
    try {
      await disconnected.future.timeout(testPingInterval * 4);
    } on TimeoutException {
      detected = false;
    }

    expect(
      detected,
      isTrue,
      reason:
          'RelaySocket must surface an unanswered ping through onDisconnected',
    );

    socket.dispose();
    await server.close();
  });

  test('keeps an idle but healthy peer connected', () async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    server.transform(WebSocketTransformer()).listen((ws) {
      // A healthy relay answers pings without sending application data.
      ws.listen((_) {}, onError: (_) {}, onDone: () {});
    });

    var tornDown = false;
    final socket = RelaySocket(
      wsUrl: 'ws://127.0.0.1:${server.port}',
      nsec: null,
      onMessage: (_) {},
      onConnected: () {},
      onDisconnected: (_) => tornDown = true,
    );
    unawaited(socket.connect());

    await Future<void>.delayed(testPingInterval * 4);

    expect(tornDown, isFalse);

    await socket.disconnect();
    await server.close(force: true);
  });
}
