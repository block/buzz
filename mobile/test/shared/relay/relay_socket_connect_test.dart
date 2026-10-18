import 'dart:async';
import 'dart:io';

import 'package:buzz/shared/relay/relay_socket.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mocktail/mocktail.dart';

class _HttpClient extends Mock implements HttpClient {}

void main() {
  test('a retired attempt closes a socket that already upgraded', () async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final peerClosed = Completer<void>();
    WebSocket? peer;
    server.listen((request) async {
      peer = await WebSocketTransformer.upgrade(request);
      peer!.listen((_) {}, onDone: peerClosed.complete);
    });
    final realClient = HttpClient();
    final client = _HttpClient();
    registerFallbackValue(Uri());
    when(() => client.openUrl(any(), any())).thenAnswer(
      (invocation) => realClient.openUrl(
        invocation.positionalArguments[0] as String,
        invocation.positionalArguments[1] as Uri,
      ),
    );
    final errors = <Object?>[];
    final socket = RelaySocket(
      wsUrl: 'ws://127.0.0.1:${server.port}',
      nsec: null,
      onMessage: (_) {},
      onConnected: () => fail('The attempt was retired before authentication'),
      onDisconnected: errors.add,
    );
    var retired = false;
    when(() => client.close(force: true)).thenAnswer((_) {
      realClient.close(force: true);
      // Retire after the real upgrade, before connect resumes authentication.
      // This makes the otherwise narrow cancellation race deterministic.
      if (!retired) {
        retired = true;
        socket.dispose();
      }
    });
    addTearDown(() async {
      socket.dispose();
      realClient.close(force: true);
      await peer?.close();
      await server.close(force: true);
    });

    await HttpOverrides.runZoned(
      socket.connect,
      createHttpClient: (_) => client,
    ).timeout(const Duration(seconds: 2));
    expect(socket.state, SocketState.disconnected);
    expect(errors, isEmpty);
    await peerClosed.future.timeout(const Duration(seconds: 2));
  });

  for (final disposeWhileConnecting in [false, true]) {
    test(
      disposeWhileConnecting
          ? 'disposing during the handshake closes the pending transport quietly'
          : 'disconnecting during the handshake closes the pending transport quietly',
      () async {
        final server = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
        final peers = <Socket>[];
        final requestReceived = Completer<void>();
        final peerClosed = Completer<void>();
        server.listen((peer) {
          peers.add(peer);
          // Accept the upgrade request but never return an HTTP response.
          peer.listen(
            (_) {
              if (!requestReceived.isCompleted) requestReceived.complete();
            },
            onDone: () {
              if (!peerClosed.isCompleted) peerClosed.complete();
            },
            onError: (Object _) {},
          );
        });
        final errors = <Object?>[];
        var connected = false;
        final socket = RelaySocket(
          wsUrl: 'ws://127.0.0.1:${server.port}',
          nsec: null,
          onMessage: (_) {},
          onConnected: () => connected = true,
          onDisconnected: errors.add,
        );
        addTearDown(() async {
          socket.dispose();
          for (final peer in peers) {
            peer.destroy();
          }
          await server.close();
        });

        final attempt = socket.connect();
        await requestReceived.future.timeout(const Duration(seconds: 2));
        expect(socket.state, SocketState.connecting);
        if (disposeWhileConnecting) {
          socket.dispose();
        } else {
          await socket.disconnect().timeout(const Duration(seconds: 2));
        }

        await attempt.timeout(const Duration(seconds: 2));
        expect(socket.state, SocketState.disconnected);
        expect(connected, isFalse);
        expect(errors, isEmpty);
        await peerClosed.future.timeout(const Duration(seconds: 2));
      },
    );
  }

  test('a retired handshake cannot disconnect its replacement', () async {
    final server = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
    final peers = <Socket>[];
    final requests = [Completer<void>(), Completer<void>()];
    server.listen((peer) {
      final received = requests[peers.length];
      peers.add(peer);
      peer.listen((_) {
        if (!received.isCompleted) received.complete();
      }, onError: (Object _) {});
    });
    final errors = <Object?>[];
    final socket = RelaySocket(
      wsUrl: 'ws://127.0.0.1:${server.port}',
      nsec: null,
      onMessage: (_) {},
      onConnected: () => fail('Neither peer completed the upgrade'),
      onDisconnected: errors.add,
    );
    addTearDown(() async {
      socket.dispose();
      for (final peer in peers) {
        peer.destroy();
      }
      await server.close();
    });

    final first = socket.connect();
    await requests[0].future.timeout(const Duration(seconds: 2));
    socket.dispose();
    final second = socket.connect();
    await requests[1].future.timeout(const Duration(seconds: 2));
    await first.timeout(const Duration(seconds: 2));
    expect(socket.state, SocketState.connecting);
    expect(errors, isEmpty);

    socket.dispose();
    await second.timeout(const Duration(seconds: 2));
    expect(socket.state, SocketState.disconnected);
    expect(errors, isEmpty);
  });
}
