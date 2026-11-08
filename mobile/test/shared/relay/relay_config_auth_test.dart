import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:buzz/shared/relay/relay_socket.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  test('configured padded key completes real NIP-42 authentication', () async {
    final keys = nostr.Keys.generate();
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    addTearDown(() => server.close(force: true));
    final received = Completer<nostr.Event>();
    server.listen((request) async {
      final peer = await WebSocketTransformer.upgrade(request);
      addTearDown(peer.close);
      peer.listen((frame) {
        final data = jsonDecode(frame as String) as List;
        if (data.first != 'AUTH') return;
        final event = nostr.Event.fromMap(
          Map<String, dynamic>.from(data[1] as Map),
        );
        received.complete(event);
        peer.add(jsonEncode(['OK', event.id, true, '']));
      });
      peer.add(jsonEncode(['AUTH', 'synthetic-challenge']));
    });
    final config = RelayConfig(
      baseUrl: 'http://127.0.0.1:${server.port}',
      nsec: '  ${keys.nsec} \n',
    );
    final socket = RelaySocket(
      wsUrl: config.wsUrl,
      nsec: config.nsec,
      onMessage: (_) {},
      onConnected: () {},
      onDisconnected: (_) {},
    );
    addTearDown(socket.dispose);
    await socket.connect().timeout(const Duration(seconds: 5));
    final event = await received.future.timeout(const Duration(seconds: 5));
    expect(socket.state, SocketState.connected);
    expect(event.pubkey, keys.public);
    expect(event.kind, 22242);
    expect(
      event.toMap()['tags'],
      contains(equals(['challenge', 'synthetic-challenge'])),
    );
    expect(event.toMap()['tags'], contains(equals(['relay', config.wsUrl])));
    expect(event.isValid(), isTrue);
  });
}
