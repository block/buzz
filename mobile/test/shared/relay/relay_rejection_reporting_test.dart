import 'dart:async';

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;

import '../diagnostics/recording_diagnostics.dart';

// What the relay's refusals do to the error list: only a reason that marks the
// request as invalid is reported, apart from the outcomes the app handles
// itself. Every other reason is counted, not reported. Reasons are the relay's
// own wording.
void main() {
  // The relay's reasons for a request it refuses as invalid.
  const reported = [
    'invalid: bad signature',
    // Mentions a timestamp, but is not about the relay's clock window.
    'invalid: feedback timestamp is out of range',
    // Limits.
    'invalid: a DM may hold at most 9 people; agents do not count',
    'invalid: content exceeds maximum size of 262144 bytes (got 262145)',
    'invalid: channel name is longer than 255 characters',
    // Push lease refusals.
    'invalid: endpoint already leased',
    'invalid: stale generation',
    'invalid: lease quota exceeded',
  ];
  // Refusals the app handles as expected outcomes, and every other reason.
  const counted = [
    // A timestamp outside the relay's window, in each wording the relay uses.
    'invalid: event timestamp too far from server time',
    'invalid: canvas event timestamp too far in the future',
    'invalid: event timestamp out of range: created_at=1, now=500, '
        'delta=-499s (max ±120s)',
    'invalid: leave request timestamp out of range (delta=300s, max ±120s)',
    'invalid: observer frame timestamp outside ±5 minute freshness window',
    'rate-limited: quota exceeded; retry in 7s',
    'rate-limited: too many concurrent requests',
    'restricted: only members can post in this channel',
    'blocked: not a member',
    'auth-required: not authenticated',
    'error: query timed out',
    'duplicate: already have this event',
    'something the relay says in no known form',
    '',
  ];
  final rows = <(String, bool)>[
    for (final reason in reported) (reason, true),
    for (final reason in counted) (reason, false),
  ];

  for (final (reason, isReported) in rows) {
    final expected = isReported ? 1 : 0;
    test('an OK refusal "$reason" is ${isReported ? '' : 'not '}reported', () {
      final harness = _session();
      harness.session.debugAttachSocketForTest(_Socket());

      final refused = harness.session.publish(_event('event-1'));
      harness.session.debugHandleMessage(['OK', 'event-1', false, reason]);

      expect(refused, throwsA(isA<Exception>()));
      expect(harness.reporter.errors, hasLength(expected));
      expect(harness.reporter.errors.map((e) => e.$2), [
        if (isReported) DiagnosticOperation.relayRejection,
      ]);
    });

    test(
      'a CLOSED refusal "$reason" is ${isReported ? '' : 'not '}reported',
      () {
        final harness = _session();

        harness.session.debugHandleMessage(['CLOSED', 'sub-1', reason]);

        expect(harness.reporter.errors.map((e) => e.$2), [
          if (isReported) DiagnosticOperation.relayRejection,
        ]);
      },
    );
  }

  // A reply for nothing pending, as for a typing indicator or a reply that
  // arrives after the publish timed out.
  for (final (accepted, reason, isReported) in [
    (false, 'invalid: bad signature', true),
    (false, 'blocked: not a member', false),
    (true, 'invalid: bad signature', false),
    (true, '', false),
  ]) {
    test('an unexpected OK ${accepted ? 'accepting' : 'refusing'} with '
        '"$reason" is ${isReported ? '' : 'not '}reported', () {
      final harness = _session();

      harness.session.debugHandleMessage(['OK', 'unknown', accepted, reason]);

      expect(harness.reporter.errors.map((e) => e.$2), [
        if (isReported) DiagnosticOperation.relayRejection,
      ]);
    });
  }

  test('an OK frame without a reason is not reported', () {
    final harness = _session();

    harness.session.debugHandleMessage(['OK', 'unknown', false]);

    expect(harness.reporter.errors, isEmpty);
  });
}

class _Harness {
  _Harness(this.session, this.reporter);

  final RelaySessionNotifier session;
  final RecordingDiagnostics reporter;
}

/// A session whose own key is the member reports name.
_Harness _session() {
  final keys = nostr.Keys.generate();
  final reporter = RecordingDiagnostics()..setMember(keys.public);
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
