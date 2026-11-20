import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/diagnostics/diagnostic_member_binding.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;

import '../diagnostics/recording_diagnostics.dart';

// More ways the member can differ from the key a session signs with, on top of
// relay_session_attribution_test.dart.
void main() {
  // A session without a key signs for no one, so it reports only while reports
  // name no one either.
  for (final (label, member, reported) in [
    ('a member', nostr.Keys.generate().public, false),
    ('no one', null, true),
  ]) {
    test('a session with no key is ${reported ? '' : 'not '}reported while '
        'reports name $label', () async {
      final reporter = RecordingDiagnostics()..setMember(member);
      final session = RelaySessionNotifier(
        httpClient: http_testing.MockClient(
          (_) async => http.Response('[]', 200),
        ),
      );
      final container = ProviderContainer(
        overrides: [
          authProvider.overrideWith(() => _PendingAuth()),
          relaySessionProvider.overrideWith(() => session),
          relayConfigProvider.overrideWith(() => _Config(null)),
          diagnosticReporterProvider.overrideWithValue(reporter),
        ],
      );
      addTearDown(container.dispose);
      container.read(relaySessionProvider);
      session.debugAttachSocketForTest(_Socket());

      final refused = session.publish(_event('event-1'));
      session.debugHandleMessage(['OK', 'event-1', false, 'invalid: bad']);
      await expectLater(refused, throwsA(isA<Exception>()));

      expect(reporter.operations, [
        if (reported) DiagnosticOperation.relayRejection,
      ]);
    });
  }

  // The relay answers over a real socket after the app has moved on to
  // another member.
  for (final member in NamedMember.values) {
    test('a refusal that arrives over the socket is '
        '${member.reported ? '' : 'not '}reported while reports name '
        '${member.label}', () async {
      final answer = Completer<void>();
      final server = await loopbackRelay(
        onFrame: (socket, frame) async {
          if (frame.first != 'EVENT') return;
          final id = (frame[1] as Map<String, dynamic>)['id'];
          await answer.future;
          socket.add(jsonEncode(['OK', id, false, 'invalid: bad signature']));
        },
      );
      final keys = nostr.Keys.generate();
      final reporter = RecordingDiagnostics()..setMember(keys.public);
      final connected = Completer<void>();
      final session = RelaySessionNotifier(
        httpClient: http_testing.MockClient(
          (_) async => http.Response('[]', 200),
        ),
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
              onMessage: onMessage,
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
          relayConfigProvider.overrideWith(() => _Config(keys.nsec)),
          authProvider.overrideWith(() => _Authenticated(keys.public)),
          diagnosticReporterProvider.overrideWithValue(reporter),
        ],
      );
      addTearDown(container.dispose);
      await container.read(authProvider.future);
      final listener = container.listen(relaySessionProvider, (_, _) {});
      addTearDown(listener.close);
      await connected.future;

      final refused = session.publish(_event('event-1'));
      await Future<void>.delayed(const Duration(milliseconds: 50));
      reporter.setMember(member.pubkey(keys.public));
      answer.complete();
      await expectLater(refused, throwsA(isA<Exception>()));

      expect(reporter.operations, [
        if (member.reported) DiagnosticOperation.relayRejection,
      ]);
    });
  }

  // Through the providers the app runs: a switch changes the key and, a
  // moment later, the member. Reports from the new session follow the new
  // member; none names the old one.
  test('after a switch the new session reports under the new member', () async {
    final reporter = RecordingDiagnostics();
    final container = ProviderContainer(
      overrides: [
        authProvider.overrideWith(() => _PendingAuth()),
        diagnosticReporterProvider.overrideWithValue(reporter),
        activeCommunityProvider.overrideWith((ref) async => null),
      ],
    );
    addTearDown(container.dispose);
    container.listen(diagnosticMemberProvider, (_, _) {});
    container.listen(relaySessionProvider, (_, _) {});
    final first = nostr.Keys.generate();
    final second = nostr.Keys.generate();
    Future<void> refuse(String id) async {
      final session = container.read(relaySessionProvider.notifier);
      session.debugAttachSocketForTest(_Socket());
      final refused = session.publish(_event(id));
      session.debugHandleMessage(['OK', id, false, 'invalid: bad']);
      await expectLater(refused, throwsA(isA<Exception>()));
    }

    final config = container.read(relayConfigProvider.notifier);
    config.update(baseUrl: 'https://relay.example', nsec: first.nsec);
    container.read(diagnosticMemberProvider);
    await refuse('one');
    expect(reporter.operations, hasLength(1));

    config.update(baseUrl: 'https://relay.example', nsec: second.nsec);
    container.read(diagnosticMemberProvider);
    await refuse('two');

    expect(reporter.member, second.public);
    expect(reporter.operations, hasLength(2));
  });
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

  final String? _nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: _nsec);
}

class _PendingAuth extends AuthNotifier {
  @override
  Future<AuthState> build() => Completer<AuthState>().future;
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
