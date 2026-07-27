import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:pointycastle/digests/sha1.dart';
import 'package:buzz/shared/relay/relay_websocket_connector.dart';

/// A WebSocket server that completes the handshake and then goes silent —
/// it never answers a ping, and never sends a close frame.
///
/// This is the shape of the failure being fixed: a carrier NAT or a dozing
/// radio drops the flow without either peer sending a FIN, so the socket looks
/// open to the client forever. It is built on a raw [ServerSocket] because
/// `dart:io`'s own `WebSocket` auto-replies to pings, which is precisely the
/// behaviour we need to withhold.
class _SilentWebSocketServer {
  _SilentWebSocketServer._(this._server);

  final ServerSocket _server;
  final List<Socket> _clients = [];

  static const _handshakeGuid = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';

  int get port => _server.port;

  Uri get uri => Uri.parse('ws://127.0.0.1:$port');

  static Future<_SilentWebSocketServer> start() async {
    final server = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
    final instance = _SilentWebSocketServer._(server);
    server.listen(instance._handleClient);
    return instance;
  }

  void _handleClient(Socket socket) {
    _clients.add(socket);
    final buffer = StringBuffer();
    var upgraded = false;

    socket.listen(
      (data) {
        // Once upgraded, every inbound frame — including pings — is discarded.
        if (upgraded) return;

        buffer.write(latin1.decode(data));
        final request = buffer.toString();
        if (!request.contains('\r\n\r\n')) return;

        final key = RegExp(
          r'sec-websocket-key:\s*(\S+)',
          caseSensitive: false,
        ).firstMatch(request)?.group(1);
        if (key == null) {
          socket.destroy();
          return;
        }

        final accept = base64.encode(
          SHA1Digest().process(latin1.encode('$key$_handshakeGuid')),
        );
        socket.add(
          latin1.encode(
            'HTTP/1.1 101 Switching Protocols\r\n'
            'Upgrade: websocket\r\n'
            'Connection: Upgrade\r\n'
            'Sec-WebSocket-Accept: $accept\r\n'
            '\r\n',
          ),
        );
        upgraded = true;
      },
      onError: (_) {},
      cancelOnError: true,
    );
  }

  Future<void> close() async {
    for (final client in _clients) {
      client.destroy();
    }
    await _server.close();
  }
}

void main() {
  test(
    'closes a silent socket once a ping goes unanswered',
    () async {
      final server = await _SilentWebSocketServer.start();
      addTearDown(server.close);

      final channel = connectRelayWebSocket(
        server.uri,
        pingInterval: const Duration(milliseconds: 200),
      );
      await channel.ready;

      final done = Completer<void>();
      Object? streamError;
      channel.stream.listen(
        (_) {},
        onError: (Object error) {
          streamError = error;
          if (!done.isCompleted) done.complete();
        },
        onDone: () {
          if (!done.isCompleted) done.complete();
        },
      );

      // The socket is healthy at the TCP level and the server never hangs up,
      // so without client-side ping/pong this never completes.
      await done.future.timeout(
        const Duration(seconds: 5),
        onTimeout: () => fail(
          'socket stayed open with no pong — client-side liveness probing is '
          'not active',
        ),
      );

      // RelaySocket distinguishes an unanswered ping from a transport failure:
      // the former must arrive as a clean close through `onDone`, which is the
      // path that drives reconnection.
      expect(
        streamError,
        isNull,
        reason: 'unanswered ping should close cleanly, not raise onError',
      );
      expect(
        channel.closeCode,
        WebSocketStatus.goingAway,
        reason: 'dart:io closes a ping-timed-out socket with goingAway (1001)',
      );
    },
    // Guards against a regression silently reintroducing an unbounded wait.
    timeout: const Timeout(Duration(seconds: 20)),
  );

  test('defaults to a ping interval inside the relay heartbeat window', () {
    // The relay closes after 3 missed 30s pongs; probing must be frequent
    // enough to detect a dead socket well before that.
    expect(relayPingInterval, lessThan(const Duration(seconds: 30)));
    expect(relayPingInterval, greaterThan(Duration.zero));
  });
}
