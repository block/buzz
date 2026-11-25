import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay_call_error.dart';
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

        // A refusal is definite: the relay saw the event and didn't store it.
        await expectLater(
          submitSignedEventOnce(
            wsUrl: 'ws://127.0.0.1:${server.port}',
            nsec: keys.nsec,
            kind: 1,
            content: '',
            tags: const [],
            diagnostics: reporter,
          ),
          throwsA(
            allOf(isA<Exception>(), isNot(isA<SignedEventUnacknowledged>())),
          ),
        );

        expect(reporter.errors, [
          if (reported)
            (RelayInvalidRequest, DiagnosticOperation.relayRejection),
        ]);
      },
    );
  }

  // Whether a failure might have reached the relay decides whether a caller
  // can treat the event as never stored. Each relay stalls at one step, and
  // the attempt ends at its deadline.
  for (final (stall, maybeStored) in [
    ('auth', false),
    ('ok', true),
    ('close after event', true),
  ]) {
    test('a relay that stalls at $stall ends at the deadline, '
        '${maybeStored ? 'maybe stored' : 'never sent'}', () async {
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      addTearDown(() => server.close(force: true));
      final events = <Object?>[];
      server.listen((request) async {
        final ws = await WebSocketTransformer.upgrade(request);
        ws.add(jsonEncode(['AUTH', 'challenge']));
        ws.listen((raw) {
          final frame = jsonDecode(raw as String) as List<dynamic>;
          final id = (frame[1] as Map<String, dynamic>)['id'];
          if (frame[0] == 'AUTH' && stall != 'auth') {
            ws.add(jsonEncode(['OK', id, true, '']));
          }
          if (frame[0] == 'EVENT') {
            events.add(id);
            if (stall == 'close after event') unawaited(ws.close());
          }
        });
      });
      final started = DateTime.now();

      final failure = submitSignedEventOnce(
        wsUrl: 'ws://127.0.0.1:${server.port}',
        nsec: nostr.Keys.generate().nsec,
        kind: 1,
        content: '',
        tags: const [],
        deadline: started.add(const Duration(milliseconds: 500)),
      );

      await expectLater(
        failure,
        throwsA(
          maybeStored
              ? isA<SignedEventUnacknowledged>()
              : allOf(
                  isA<TimeoutException>(),
                  isNot(isA<SignedEventUnacknowledged>()),
                ),
        ),
      );
      expect(
        DateTime.now().difference(started),
        lessThan(const Duration(seconds: 3)),
      );
      expect(events, hasLength(maybeStored ? 1 : 0));
    });
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
  test('a deadline that passes while authenticating reaches the caller, '
      'and the event is never sent', () async {
    final events = <Object?>[];
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    addTearDown(() => server.close(force: true));
    server.listen((request) async {
      final ws = await WebSocketTransformer.upgrade(request);
      ws.add(jsonEncode(['AUTH', 'challenge']));
      ws.listen((raw) {
        final frame = jsonDecode(raw as String) as List<dynamic>;
        final id = (frame[1] as Map<String, dynamic>)['id'];
        if (frame[0] == 'EVENT') events.add(id);
        if (frame[0] == 'AUTH') {
          // Authentication answers only after the caller's deadline.
          Timer(const Duration(milliseconds: 600), () {
            ws.add(jsonEncode(['OK', id, true, '']));
          });
        }
      });
    });
    final watch = Stopwatch()..start();

    await expectLater(
      submitSignedEventOnce(
        wsUrl: 'ws://127.0.0.1:${server.port}',
        nsec: nostr.Keys.generate().nsec,
        kind: 1,
        content: '',
        tags: const [],
        timeout: const Duration(milliseconds: 200),
      ),
      throwsA(
        isA<RelayTimeoutException>().having(
          (error) => error.call,
          'call',
          RelayCall.publish,
        ),
      ),
    );
    expect(watch.elapsed, lessThan(const Duration(milliseconds: 550)));
    // Outlive the late authentication: an uncaught error would fail the test.
    await Future<void>.delayed(const Duration(milliseconds: 800));

    expect(events, isEmpty);
  });
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
