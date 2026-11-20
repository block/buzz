import 'dart:async';
import 'dart:io';

import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay_socket.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('reports a frame it cannot parse and keeps reading', () async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    addTearDown(() => server.close(force: true));
    server.listen((request) async {
      final ws = await WebSocketTransformer.upgrade(request);
      ws.add('not json');
      ws.add('["NOTICE","still here"]');
    });

    final reporter = _RecordingDiagnostics();
    final received = Completer<List<dynamic>>();
    final socket = RelaySocket(
      wsUrl: 'ws://127.0.0.1:${server.port}',
      nsec: null,
      onMessage: (message) {
        if (!received.isCompleted) received.complete(message);
      },
      onConnected: () {},
      onDisconnected: (_) {},
    )..diagnostics = reporter;
    addTearDown(socket.dispose);
    unawaited(socket.connect());

    expect(await received.future, ['NOTICE', 'still here']);
    expect(reporter.errors, [
      (FormatException, DiagnosticOperation.relayFrame),
    ]);
  });
}

class _RecordingDiagnostics extends NoopDiagnosticReporter {
  final errors = <(Type, DiagnosticOperation)>[];

  @override
  void reportError(
    Object error,
    StackTrace? stackTrace, {
    required DiagnosticOperation operation,
  }) {
    errors.add((error.runtimeType, operation));
  }
}
