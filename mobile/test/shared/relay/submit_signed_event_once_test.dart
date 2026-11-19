import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay_closed_policy.dart';
import 'package:buzz/shared/relay/signed_event_relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  // A relay that authenticates the socket, then answers the event with
  // [reason].
  Future<HttpServer> relayRefusing(String reason) async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    addTearDown(() => server.close(force: true));
    server.listen((request) async {
      final ws = await WebSocketTransformer.upgrade(request);
      ws.add(jsonEncode(['AUTH', 'challenge']));
      ws.listen((raw) {
        final frame = jsonDecode(raw as String) as List<dynamic>;
        final id = (frame[1] as Map<String, dynamic>)['id'];
        if (frame[0] == 'AUTH') ws.add(jsonEncode(['OK', id, true, '']));
        if (frame[0] == 'EVENT') ws.add(jsonEncode(['OK', id, false, reason]));
      });
    });
    return server;
  }

  for (final (reason, reported) in [
    ('invalid: bad signature', true),
    ('invalid: event timestamp too far from server time', false),
    ('blocked: not a member', false),
  ]) {
    test(
      'a refusal "$reason" ${reported ? 'is' : 'is not'} reported',
      () async {
        final server = await relayRefusing(reason);
        final keys = nostr.Keys.generate();
        final reporter = _RecordingDiagnostics()..setMember(keys.public);

        await expectLater(
          submitSignedEventOnce(
            wsUrl: 'ws://127.0.0.1:${server.port}',
            nsec: keys.nsec,
            kind: 1,
            content: '',
            tags: const [],
            diagnostics: reporter,
          ),
          throwsA(isA<Exception>()),
        );

        expect(reporter.errors, [
          if (reported)
            (RelayInvalidRequest, DiagnosticOperation.relayRejection),
        ]);
      },
    );
  }

  // A removed community's tombstone, say, signs with a key other than the
  // active member's, and its report would name the wrong member. With no
  // member set, as on first run, reports carry the install identifier.
  for (final (member, reported) in [
    (nostr.Keys.generate().public, false),
    (null, true),
  ]) {
    test('a refusal while the member is ${member ?? 'unset'} '
        '${reported ? 'is' : 'is not'} reported', () async {
      final server = await relayRefusing('invalid: bad signature');
      final reporter = _RecordingDiagnostics()..setMember(member);

      await expectLater(
        submitSignedEventOnce(
          wsUrl: 'ws://127.0.0.1:${server.port}',
          nsec: nostr.Keys.generate().nsec,
          kind: 1,
          content: '',
          tags: const [],
          diagnostics: reporter,
        ),
        throwsA(isA<Exception>()),
      );

      expect(reporter.errors, [
        if (reported) (RelayInvalidRequest, DiagnosticOperation.relayRejection),
      ]);
    });
  }
}

class _RecordingDiagnostics extends NoopDiagnosticReporter {
  final errors = <(Type, DiagnosticOperation)>[];
  String? _member;

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
