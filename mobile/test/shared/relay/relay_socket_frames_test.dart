import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay_socket.dart';
import 'package:flutter_test/flutter_test.dart';

// A frame the app cannot decode is reported, once, and the socket keeps
// reading. A frame it can decode is not reported, whatever it says.
void main() {
  for (final (name, frame, reported) in <(String, Object, bool)>[
    ('not json', 'not json', true),
    ('a truncated frame', '["EVENT","sub",{"id":"ab', true),
    ('an empty frame', '', true),
    ('an object where an array belongs', '{"error":"nope"}', true),
    ('a JSON string', '"hello"', true),
    ('a binary frame', utf8.encode('["NOTICE","binary"]'), false),
    ('an empty array', '[]', false),
    ('a frame of a type the app ignores', '["NOTICE","hello"]', false),
  ]) {
    test(
      '$name is ${reported ? '' : 'not '}reported and reading goes on',
      () async {
        final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
        addTearDown(() => server.close(force: true));
        server.listen((request) async {
          final ws = await WebSocketTransformer.upgrade(request);
          ws
            ..add(frame)
            ..add('["NOTICE","still here"]');
        });

        final reporter = _RecordingDiagnostics();
        final received = Completer<void>();
        final socket = RelaySocket(
          wsUrl: 'ws://127.0.0.1:${server.port}',
          nsec: null,
          onMessage: (message) {
            if (message.last == 'still here' && !received.isCompleted) {
              received.complete();
            }
          },
          onConnected: () {},
          onDisconnected: (_) {},
        )..diagnostics = reporter;
        addTearDown(socket.dispose);
        unawaited(socket.connect());

        await received.future;
        expect(reporter.operations, [
          if (reported) DiagnosticOperation.relayFrame,
        ]);
      },
    );
  }
}

class _RecordingDiagnostics extends NoopDiagnosticReporter {
  final operations = <DiagnosticOperation>[];

  @override
  void reportError(
    Object error,
    StackTrace? stackTrace, {
    required DiagnosticOperation operation,
  }) {
    operations.add(operation);
  }
}
