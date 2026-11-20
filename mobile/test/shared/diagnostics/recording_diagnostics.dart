import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:flutter_test/flutter_test.dart';

/// A reporter that records what it was given: the error's type and the
/// operation. The member is whatever the test last named.
class RecordingDiagnostics extends NoopDiagnosticReporter {
  final errors = <(Type, DiagnosticOperation)>[];
  String? _member;

  List<DiagnosticOperation> get operations => [
    for (final (_, operation) in errors) operation,
  ];

  @override
  String? get member => _member;

  @override
  void setMember(String? pubkey) => _member = pubkey;

  @override
  void reportError(
    Object error,
    StackTrace? stackTrace, {
    required DiagnosticOperation operation,
  }) => errors.add((error.runtimeType, operation));
}

/// Who a report names while a request signed by one key is in flight.
enum NamedMember {
  other('another member'),
  signer('the signer'),
  none('no one');

  const NamedMember(this.label);
  final String label;

  /// Whether a report from work signed by [signer] goes out.
  bool get reported => this != other;

  String? pubkey(String signer) => switch (this) {
    other => 'e' * 64,
    NamedMember.signer => signer,
    none => null,
  };
}

/// A loopback relay that sends [first] as raw frames (text or binary) as soon as a
/// client connects, then the NIP-42 challenge, and accepts the AUTH it gets
/// back. [onFrame] sees every other frame the client sends, with the socket.
Future<HttpServer> loopbackRelay({
  List<Object> first = const [],
  void Function(WebSocket socket, List<dynamic> frame)? onFrame,
  bool challenge = true,
  bool closeAfterFirst = false,
}) async {
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
  addTearDown(() => server.close(force: true));
  server.listen((request) async {
    final ws = await WebSocketTransformer.upgrade(request);
    for (final frame in first) {
      ws.add(frame);
    }
    if (closeAfterFirst) {
      await ws.close();
      return;
    }
    ws.listen((raw) {
      final frame = jsonDecode(raw as String) as List<dynamic>;
      if (frame.first == 'AUTH') {
        final id = (frame[1] as Map<String, dynamic>)['id'];
        ws.add(jsonEncode(['OK', id, true, '']));
      } else {
        onFrame?.call(ws, frame);
      }
    });
    if (challenge) ws.add(jsonEncode(['AUTH', 'challenge']));
  });
  return server;
}
