import 'dart:async';
import 'dart:io';

import 'package:buzz/shared/relay/relay_socket.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
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
