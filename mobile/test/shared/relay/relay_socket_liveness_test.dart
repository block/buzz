import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/relay/relay_socket.dart';
import 'package:buzz/shared/relay/relay_websocket.dart';
import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';

/// A server that completes the WS handshake then never speaks again: no pongs,
/// no close frame. Only a client-side ping timeout can notice.
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
          'Upgrade: websocket\r\nConnection: Upgrade\r\n'
          'Sec-WebSocket-Accept: $accept\r\n\r\n',
        );
      },
      onError: (_) {},
      onDone: () {},
    );
  });
  return server;
}

/// A server that accepts TCP connections and then never answers, like a
/// stalled TLS or upgrade handshake on a congested network.
/// Accepts TCP and never answers the upgrade. [closed] completes when the
/// client drops the connection.
Future<(ServerSocket server, Future<void> closed)>
_stalledHandshakeServer() async {
  final server = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
  final closed = Completer<void>();
  server.listen(
    (client) => client.listen(
      (_) {},
      onError: (_) {},
      onDone: () {
        if (!closed.isCompleted) closed.complete();
      },
    ),
  );
  return (server, closed.future);
}

void main() {
  const testPingInterval = Duration(milliseconds: 150);
  const testConnectTimeout = Duration(milliseconds: 200);

  setUp(() {
    RelaySocket.debugPingInterval = testPingInterval;
    RelaySocket.debugConnectTimeout = testConnectTimeout;
  });

  tearDown(() {
    RelaySocket.debugPingInterval = RelaySocket.pingInterval;
    RelaySocket.debugConnectTimeout = relayWebSocketConnectTimeout;
  });

  for (final scoped in [false, true]) {
    test('gives up on a handshake that never completes '
        '(${scoped ? 'scoped headers' : 'no headers'})', () async {
      final (server, clientClosed) = await _stalledHandshakeServer();

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
      if (scoped) {
        socket.headers = {'Nostr-Federated-Identity': 'Bearer synthetic'};
      }
      unawaited(socket.connect());

      final error = await disconnected.future.timeout(
        testConnectTimeout * 10,
        onTimeout: () => fail(
          'RelaySocket must fail a stalled handshake so the session can '
          'schedule a reconnect',
        ),
      );
      expect(socket.state, SocketState.disconnected);
      expect(error.toString(), contains('Relay connection timed out'));
      // The abandoned handshake is closed, not left for the OS to time out.
      await clientClosed.timeout(
        testConnectTimeout * 10,
        onTimeout: () => fail('the timed-out handshake was left open'),
      );

      socket.dispose();
      await server.close();
    });
  }

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

  test(
    'scoped headers stay at the upgrade origin and healthy peers stay connected',
    () async {
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      final paths = <String>[];
      // A different port prevents a closed upgrade connection from masking a
      // redirect attempt through connection-pool reuse.
      final sink = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      sink.listen((request) async {
        paths.add('sink');
        expect(
          request.headers.value('Nostr-Federated-Identity'),
          'Bearer synthetic',
        );
        request.response.statusCode = 403;
        await request.response.close();
      });
      server.listen((request) async {
        paths.add(request.uri.path);
        expect(
          request.headers.value('Nostr-Federated-Identity'),
          'Bearer synthetic',
        );
        if (request.uri.path == '/denied') {
          request.response.statusCode = 403;
          await request.response.close();
          return;
        }
        if (request.uri.path == '/redirect') {
          request.response.statusCode = 302;
          request.response.headers.set(
            'Location',
            'http://127.0.0.1:${sink.port}/sink',
          );
          await request.response.close();
          return;
        }
        final ws = await WebSocketTransformer.upgrade(request);
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
      socket.headers = {'Nostr-Federated-Identity': 'Bearer synthetic'};
      unawaited(socket.connect());

      await Future<void>.delayed(testPingInterval * 4);

      expect(tornDown, isFalse);

      await socket.disconnect();
      final refused = Completer<void>();
      final redirect = RelaySocket(
        wsUrl: 'ws://127.0.0.1:${server.port}/redirect',
        nsec: null,
        onMessage: (_) {},
        onConnected: () {},
        onDisconnected: (_) {
          if (!refused.isCompleted) refused.complete();
        },
      );
      redirect.headers = {'Nostr-Federated-Identity': 'Bearer synthetic'};
      await redirect.connect();
      await refused.future;
      expect(paths, ['/', '/redirect']);
      redirect.dispose();
      final denial = Completer<Object?>();
      final denied = RelaySocket(
        wsUrl: 'ws://127.0.0.1:${server.port}/denied',
        nsec: null,
        onMessage: (_) {},
        onConnected: () {},
        onDisconnected: (error) {
          if (!denial.isCompleted) denial.complete(error);
        },
      );
      denied.headers = {'Nostr-Federated-Identity': 'Bearer synthetic'};
      await denied.connect();
      expect(relayUpgradeRefused(await denial.future), isTrue);
      denied.dispose();
      await server.close(force: true);
      await sink.close(force: true);
    },
  );
}
