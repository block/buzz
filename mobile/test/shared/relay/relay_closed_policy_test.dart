import 'dart:async';

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;

void main() {
  group('classifyRelayClosed', () {
    const cases = {
      'rate-limited: quota exceeded; retry in 4s': RelayClosedClass.rateLimited,
      'rate-limited: too many concurrent requests':
          RelayClosedClass.rateLimited,
      'restricted: access revoked': RelayClosedClass.terminal,
      'auth-required: verification failed': RelayClosedClass.terminal,
      'blocked: community policy': RelayClosedClass.terminal,
      'invalid: malformed filter': RelayClosedClass.terminal,
      'pow: insufficient work': RelayClosedClass.terminal,
      'duplicate: subscription already exists': RelayClosedClass.terminal,
      'unsupported: filter extension': RelayClosedClass.terminal,
      'error: mixed search and channel filter': RelayClosedClass.terminal,
      'error: too many subscriptions': RelayClosedClass.terminal,
      'error: query timed out': RelayClosedClass.terminal,
      'error: relay temporarily unavailable': RelayClosedClass.retryable,
      'subscription closed by relay': RelayClosedClass.retryable,
    };

    for (final entry in cases.entries) {
      test(entry.key, () {
        expect(classifyRelayClosed(entry.key), entry.value);
      });
    }
  });

  group('isInvalidRequestRejection', () {
    const cases = {
      'invalid: bad signature': true,
      'INVALID: malformed filter': true,
      'invalid: channel is archived': false,
      'invalid: channel is not archived': false,
      'invalid: an open channel named "general" already exists': false,
      // The relay's refusals for a timestamp outside its clock window, in each
      // wording it uses, are the app's own clock and not reported.
      'invalid: event timestamp too far from server time': false,
      'invalid: canvas event timestamp too far in the future': false,
      'invalid: event timestamp out of range: created_at=1, now=500, '
              'delta=-499s (max \u00b1120s)':
          false,
      'invalid: leave request timestamp out of range (delta=300s, max \u00b1120s)':
          false,
      'invalid: observer frame timestamp outside \u00b15 minute freshness window':
          false,
      // Mentions a timestamp, but is not about the relay's clock window.
      'invalid: feedback timestamp is out of range': true,
      // Echoes a value that is a clock-window wording.
      'invalid: unsupported report type: invalid: event timestamp too far from server time':
          true,
      'invalid: policy:nobody \u2014 this agent has disabled external channel additions':
          false,
      'invalid: policy:owner_only': false,
      'invalid: cannot remove the last owner': false,
      'invalid: only members can add people in this channel': false,
      'duplicate: channel already exists': false,
      'restricted: access revoked': false,
      'rate-limited: quota exceeded; retry in 4s': false,
      'error: query timed out': false,
      '': false,
    };

    for (final entry in cases.entries) {
      test(entry.key.isEmpty ? '(empty)' : entry.key, () {
        expect(isInvalidRequestRejection(entry.key), entry.value);
      });
    }
  });

  // A refusal reaches the error list through the session, only when
  // isInvalidRequestRejection says so.
  group('relay refusals reported by the session', () {
    List<DiagnosticOperation> operations(void Function(_Harness) receive) {
      final harness = _session();
      receive(harness);
      return [for (final (_, operation) in harness.reporter.errors) operation];
    }

    test('an OK refusal is reported as the request it refused', () {
      final harness = _session();
      harness.session.debugAttachSocketForTest(_Socket());

      final refused = harness.session.publish(_event('event-1'));
      harness.session.debugHandleMessage([
        'OK',
        'event-1',
        false,
        'invalid: bad signature',
      ]);

      expect(refused, throwsA(isA<Exception>()));
      expect(harness.reporter.errors.map((e) => e.$2), [
        DiagnosticOperation.relayRejection,
      ]);
    });

    test('a CLOSED refusal is reported', () {
      expect(
        operations(
          (h) => h.session.debugHandleMessage([
            'CLOSED',
            'sub-1',
            'invalid: bad signature',
          ]),
        ),
        [DiagnosticOperation.relayRejection],
      );
    });

    // A reply for nothing pending, as for a typing indicator or a reply that
    // arrives after the publish timed out.
    for (final (frame, reported) in <(List<Object>, bool)>[
      (['OK', 'unknown', false, 'invalid: bad signature'], true),
      (['OK', 'unknown', false, 'blocked: not a member'], false),
      (['OK', 'unknown', true, 'invalid: bad signature'], false),
      (['OK', 'unknown', false], false),
    ]) {
      test('an unexpected $frame is ${reported ? '' : 'not '}reported', () {
        expect(
          operations((h) => h.session.debugHandleMessage(frame)),
          reported ? [DiagnosticOperation.relayRejection] : isEmpty,
        );
      });
    }
  });

  test('parseRateLimitRetrySeconds handles canonical and absent hints', () {
    expect(
      parseRateLimitRetrySeconds('rate-limited: quota exceeded; retry in 17s'),
      17,
    );
    expect(parseRateLimitRetrySeconds('RETRY IN 2S'), 2);
    expect(parseRateLimitRetrySeconds('retry in 0s'), 0);
    expect(parseRateLimitRetrySeconds('retry in 999999s'), 999999);
    final oversizedHint = List.filled(1000, '9').join();
    expect(parseRateLimitRetrySeconds('retry in ${oversizedHint}s'), isNull);
    expect(
      parseRateLimitRetrySeconds('rate-limited: too many concurrent requests'),
      isNull,
    );
  });

  group('relayErrorMessage', () {
    // Messages as relay_session completes a rejected publish: Exception(message).
    const cases = {
      'invalid: an open channel named "Launch" already exists':
          'An open channel named "Launch" already exists',
      'blocked: community policy': 'Community policy',
      'error: internal failure': 'Internal failure',
      'rate-limited: slow down, retry in 3s': 'Slow down, retry in 3s',
      'auth-required: sign in first': 'Sign in first',
      'relay unavailable': 'Relay unavailable',
      'Event rejected': 'Event rejected',
    };
    for (final entry in cases.entries) {
      test(entry.key, () {
        expect(relayErrorMessage(Exception(entry.key)), entry.value);
      });
    }

    test('a bare string error and an empty remainder stay readable', () {
      expect(relayErrorMessage('invalid: bad name'), 'Bad name');
      expect(relayErrorMessage(Exception('')), 'Exception:');
    });
  });
}

class _Harness {
  _Harness(this.session, this.reporter);

  final RelaySessionNotifier session;
  final _RecordingDiagnostics reporter;
}

/// A session whose own key is the member reports name.
_Harness _session() {
  final keys = nostr.Keys.generate();
  final reporter = _RecordingDiagnostics()..setMember(keys.public);
  final session = RelaySessionNotifier(
    httpClient: http_testing.MockClient((_) async => http.Response('[]', 200)),
  );
  final container = ProviderContainer(
    overrides: [
      authProvider.overrideWith(() => _PendingAuth()),
      relaySessionProvider.overrideWith(() => session),
      relayConfigProvider.overrideWith(() => _Config(keys.nsec)),
      diagnosticReporterProvider.overrideWithValue(reporter),
    ],
  );
  addTearDown(container.dispose);
  container.read(relaySessionProvider);
  return _Harness(session, reporter);
}

NostrEvent _event(String id) => NostrEvent(
  id: id,
  pubkey: 'alice',
  createdAt: 20,
  kind: 1,
  tags: const [],
  content: 'hello',
  sig: 'sig',
);

class _Config extends RelayConfigNotifier {
  _Config(this._nsec);

  final String _nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: _nsec);
}

class _PendingAuth extends AuthNotifier {
  @override
  Future<AuthState> build() => Completer<AuthState>().future;
}

class _Socket extends RelaySocket {
  _Socket()
    : super(
        wsUrl: 'wss://relay.example',
        nsec: null,
        onMessage: (_) {},
        onConnected: () {},
        onDisconnected: (_) {},
      );

  @override
  void send(List<dynamic> payload) {}

  @override
  void dispose() {}
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
