import 'dart:async';
import 'dart:io';

import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:buzz/shared/relay/relay_socket.dart';
import 'package:buzz/shared/relay/signed_event_relay.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

class _ControlledSocket extends RelaySocket {
  final Completer<void>? connectionGate;
  final void Function() connected;
  final void Function(List<dynamic>) message;
  bool disconnected = false;
  bool eventSent = false;
  bool acknowledge = false;

  _ControlledSocket({
    required super.wsUrl,
    required super.nsec,
    required super.onMessage,
    required super.onConnected,
    required super.onDisconnected,
    this.connectionGate,
  }) : connected = onConnected,
       message = onMessage;

  @override
  Future<void> connect() async {
    if (connectionGate != null) await connectionGate!.future;
    if (!disconnected) connected();
  }

  @override
  void send(List<dynamic> payload) {
    eventSent = true;
    if (acknowledge) {
      final event = payload[1] as Map<String, dynamic>;
      message(['OK', event['id'], true, 'saved']);
    }
  }

  @override
  Future<void> disconnect() async {
    disconnected = true;
    if (connectionGate != null && !connectionGate!.isCompleted) {
      connectionGate!.complete();
    }
  }
}

void main() {
  final key = nostr.Keys.generate();
  const timeout = Duration(seconds: 12);

  test('connection timeout closes a stalled WebSocket handshake', () async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    server.listen((request) {});
    try {
      await expectLater(
        submitSignedEventOnce(
          wsUrl: 'ws://127.0.0.1:${server.port}',
          nsec: key.nsec,
          kind: 30350,
          content: 'inactive fixture',
          tags: const [],
          socketFactory:
              ({
                required wsUrl,
                required nsec,
                required onMessage,
                required onConnected,
                required onDisconnected,
              }) => RelaySocket(
                wsUrl: wsUrl,
                nsec: nsec,
                onMessage: onMessage,
                onConnected: onConnected,
                onDisconnected: onDisconnected,
                connectTimeout: const Duration(milliseconds: 50),
              ),
        ).timeout(const Duration(seconds: 2)),
        throwsA(
          isA<TimeoutException>().having(
            (error) => error.duration,
            'connection deadline',
            const Duration(milliseconds: 50),
          ),
        ),
      );
    } finally {
      await server.close(force: true);
    }
  });

  test('one-shot deadline includes a stalled connection', () {
    fakeAsync((clock) {
      final gate = Completer<void>();
      late _ControlledSocket socket;
      final errors = <Object>[];
      final publication = submitSignedEventOnce(
        wsUrl: 'wss://relay.example',
        nsec: key.nsec,
        kind: 30350,
        content: 'inactive fixture',
        tags: const [],
        timeout: timeout,
        socketFactory:
            ({
              required wsUrl,
              required nsec,
              required onMessage,
              required onConnected,
              required onDisconnected,
            }) => socket = _ControlledSocket(
              wsUrl: wsUrl,
              nsec: nsec,
              onMessage: onMessage,
              onConnected: onConnected,
              onDisconnected: onDisconnected,
              connectionGate: gate,
            ),
      );
      unawaited(
        publication.then<void>((_) {}, onError: (Object e) => errors.add(e)),
      );
      clock.flushMicrotasks();
      try {
        clock.elapse(timeout);
        clock.flushMicrotasks();
        expect(errors, [isA<TimeoutException>()]);
        expect(socket.disconnected, isTrue);
        expect(socket.eventSent, isFalse);
      } finally {
        if (!gate.isCompleted) gate.complete();
        clock.flushMicrotasks();
      }
    });
  });

  test('one-shot deadline bounds waiting for relay acknowledgement', () {
    fakeAsync((clock) {
      late _ControlledSocket socket;
      final errors = <Object>[];
      unawaited(
        submitSignedEventOnce(
          wsUrl: 'wss://relay.example',
          nsec: key.nsec,
          kind: 30350,
          content: 'inactive fixture',
          tags: const [],
          socketFactory:
              ({
                required wsUrl,
                required nsec,
                required onMessage,
                required onConnected,
                required onDisconnected,
              }) => socket = _ControlledSocket(
                wsUrl: wsUrl,
                nsec: nsec,
                onMessage: onMessage,
                onConnected: onConnected,
                onDisconnected: onDisconnected,
              ),
        ).then<void>((_) {}, onError: (Object e) => errors.add(e)),
      );
      clock.flushMicrotasks();
      expect(socket.eventSent, isTrue);
      clock.elapse(timeout);
      expect(errors, [isA<TimeoutException>()]);
      expect(socket.disconnected, isTrue);
    });
  });

  test('accepted one-shot publication closes its temporary socket', () {
    fakeAsync((clock) {
      late _ControlledSocket socket;
      final accepted = <NostrEvent>[];
      unawaited(
        submitSignedEventOnce(
          wsUrl: 'wss://relay.example',
          nsec: key.nsec,
          kind: 30350,
          content: 'inactive fixture',
          tags: const [],
          socketFactory:
              ({
                required wsUrl,
                required nsec,
                required onMessage,
                required onConnected,
                required onDisconnected,
              }) {
                socket = _ControlledSocket(
                  wsUrl: wsUrl,
                  nsec: nsec,
                  onMessage: onMessage,
                  onConnected: onConnected,
                  onDisconnected: onDisconnected,
                )..acknowledge = true;
                return socket;
              },
        ).then(accepted.add),
      );
      clock.flushMicrotasks();
      expect(accepted.single.content, 'saved');
      expect(socket.disconnected, isTrue);
    });
  });
}
