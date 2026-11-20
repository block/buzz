import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;

// A community switch names the next member while the old session is still
// live, so each report is checked against the member when it is made.
void main() {
  for (final (label, member, reported) in [
    ('another member', _Member.other, false),
    ('its own key', _Member.signer, true),
    ('no one', _Member.none, true),
  ]) {
    test('a session reports only while $label is the member', () async {
      final harness = await _connectedSession();
      final session = harness.session;
      final reporter = harness.reporter;
      void nameMember() => reporter.setMember(switch (member) {
        _Member.other => nostr.Keys.generate().public,
        _Member.signer => harness.keys.public,
        _Member.none => null,
      });

      // Each request starts while the session's own key is the member.
      final rejected = session.publish(_event('event-1'));
      final query = session.queryRelay(const [
        NostrFilter(kinds: [1]),
      ]);
      await Future<void>.delayed(Duration.zero);
      nameMember();

      session.debugHandleSocketMessageForTest([
        'OK',
        'event-1',
        false,
        'invalid: bad signature',
      ]);
      await expectLater(rejected, throwsA(isA<Exception>()));
      harness.decode.complete();
      await expectLater(query, throwsA(isA<FormatException>()));
      harness.sendFrames();
      await harness.noticed.future;

      expect(reporter.errors, [
        if (reported) ...[
          (RelayInvalidRequest, DiagnosticOperation.relayRejection),
          (FormatException, DiagnosticOperation.relayQuery),
          (FormatException, DiagnosticOperation.relayFrame),
        ],
      ]);
    });
  }
}

enum _Member { other, signer, none }

class _Harness {
  _Harness({
    required this.session,
    required this.reporter,
    required this.keys,
    required this.decode,
    required this.noticed,
    required this.sendFrames,
  });

  final RelaySessionNotifier session;
  final _RecordingDiagnostics reporter;
  final nostr.Keys keys;
  final Completer<void> decode;
  final Completer<void> noticed;
  final void Function() sendFrames;
}

/// A session connected through a real socket to a loopback relay, with its
/// own key as the member.
Future<_Harness> _connectedSession() async {
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
  addTearDown(() => server.close(force: true));
  final clients = <WebSocket>[];
  server.listen((request) async {
    final ws = await WebSocketTransformer.upgrade(request);
    clients.add(ws);
    ws.listen((frame) {
      final message = jsonDecode(frame as String) as List<dynamic>;
      if (message.first == 'AUTH') {
        final id = (message[1] as Map<String, dynamic>)['id'];
        ws.add(jsonEncode(['OK', id, true, '']));
      }
    });
    ws.add(jsonEncode(['AUTH', 'challenge']));
  });

  final keys = nostr.Keys.generate();
  final reporter = _RecordingDiagnostics()..setMember(keys.public);
  final decode = Completer<void>();
  final noticed = Completer<void>();
  final connected = Completer<void>();
  final session = RelaySessionNotifier(
    httpClient: http_testing.MockClient((_) async => http.Response('[]', 200)),
    queryDecoder: (_) async {
      await decode.future;
      throw const FormatException('bad body');
    },
    socketFactory:
        ({
          required wsUrl,
          required nsec,
          required onMessage,
          required onConnected,
          required onDisconnected,
        }) => RelaySocket(
          wsUrl: 'ws://127.0.0.1:${server.port}',
          nsec: nsec,
          onMessage: (message) {
            if (message.first == 'NOTICE' && !noticed.isCompleted) {
              noticed.complete();
            }
            onMessage(message);
          },
          onConnected: () {
            onConnected();
            if (!connected.isCompleted) connected.complete();
          },
          onDisconnected: onDisconnected,
        ),
  );
  final container = ProviderContainer(
    overrides: [
      relaySessionProvider.overrideWith(() => session),
      relayConfigProvider.overrideWith(
        () => _RelayConfig(
          RelayConfig(baseUrl: 'https://relay.example', nsec: keys.nsec),
        ),
      ),
      authProvider.overrideWith(() => _Authenticated(keys.public)),
      diagnosticReporterProvider.overrideWithValue(reporter),
    ],
  );
  addTearDown(container.dispose);
  await container.read(authProvider.future);
  final listener = container.listen(relaySessionProvider, (_, _) {});
  addTearDown(listener.close);
  await connected.future;
  return _Harness(
    session: session,
    reporter: reporter,
    keys: keys,
    decode: decode,
    noticed: noticed,
    sendFrames: () => clients.last
      ..add('not json')
      ..add('["NOTICE","still here"]'),
  );
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

class _RelayConfig extends RelayConfigNotifier {
  _RelayConfig(this._config);

  final RelayConfig _config;

  @override
  RelayConfig build() => _config;
}

class _Authenticated extends AuthNotifier {
  _Authenticated(this._pubkey);

  final String _pubkey;

  @override
  Future<AuthState> build() async => AuthState(
    status: AuthStatus.authenticated,
    community: Community(
      id: 'community-1',
      name: 'Example',
      relayUrl: 'https://relay.example',
      pubkey: _pubkey,
      addedAt: DateTime(2026),
    ),
  );
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
