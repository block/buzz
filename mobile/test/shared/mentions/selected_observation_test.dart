import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/relay/relay_evidence_clock.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../crypto/nip_oa_test.dart' show authTag, profile;
import 'agent_policy_test.dart' show signed;

void main() {
  test(
    'coordinate eviction retires in-flight proof without global cancellation',
    () {
      final clock = RelayEvidenceClock();
      final owner = nostr.Keys.generate();
      final retained = clock.retain(0, owner.public, null);
      for (var i = 0; i < 8192; i++) {
        clock.retain(30177, owner.public, '$i');
      }
      expect(retained(), isFalse);
      final unrelated = clock.snapshot(30177, owner.public, '8191', []);
      expect(unrelated(), isTrue);
    },
  );

  for (final scenario in [
    'policy',
    'runtime',
    'missing profile',
    'roster',
    'unrelated',
    'retired session',
    'replacement socket',
    'queued policy',
    'queued missing profile',
  ]) {
    final mode = scenario.replaceFirst('queued ', '');
    final queued = scenario.startsWith('queued ');
    test(
      'production reader rejects stale completion after observed $scenario',
      () async {
        final viewer = nostr.Keys.generate();
        final agent = nostr.Keys.generate();
        final relay = nostr.Keys.generate();
        final other = nostr.Keys.generate();
        NostrEvent policy(String mode, int time) => signed(
          viewer,
          30177,
          {'name': 'Agent', 'parallelism': 1, 'respond_to': mode},
          time: time,
          tags: [
            ['d', agent.public],
          ],
        );
        NostrEvent roster(int time) => signed(
          relay,
          39002,
          '',
          time: time,
          tags: [
            ['d', 'room'],
            ['p', viewer.public],
            [
              'p',
              agent.public,
              '',
              mode == 'missing profile' ? 'member' : 'bot',
            ],
          ],
        );
        final events = [
          roster(100),
          if (mode != 'missing profile')
            profile(agent, [authTag(viewer, agent.public)]),
          if (mode != 'missing profile')
            signed(agent, 10100, {'respond_to': 'anyone'}, time: 100),
          policy('anyone', 100),
        ];
        final oldQuery = Completer<void>();
        final release = Completer<void>();
        final blockedKind = switch (mode) {
          'runtime' => 10100,
          'missing profile' => 0,
          'roster' => 39002,
          _ => 30177,
        };
        final client = MockClient((request) async {
          if (request.method == 'GET') {
            return http.Response(jsonEncode({'self': relay.public}), 200);
          }
          final filters = (jsonDecode(request.body) as List)
              .cast<Map<String, dynamic>>();
          final results = events
              .where(
                (event) => filters.any(
                  (filter) =>
                      (filter['kinds'] as List).contains(event.kind) &&
                      (filter['authors'] as List).contains(event.pubkey) &&
                      (filter['#d'] == null ||
                          (filter['#d'] as List).contains(
                            event.getTagValue('d'),
                          )),
                ),
              )
              .toList();
          if ((filters.first['kinds'] as List).contains(blockedKind)) {
            oldQuery.complete();
            await release.future;
          }
          return http.Response(
            jsonEncode(results.map((e) => e.toJson()).toList()),
            200,
          );
        });
        final gate = RelayRateLimitGate();
        final session = RelaySessionNotifier(
          httpClient: client,
          rateLimitGate: gate,
        );
        final container = ProviderContainer(
          overrides: [
            authProvider.overrideWith(_Auth.new),
            relayConfigProvider.overrideWith(() => _Config(viewer.nsec)),
            relaySessionProvider.overrideWith(() => session),
          ],
        );
        addTearDown(container.dispose);
        await container.read(authProvider.future);
        container.read(relaySessionProvider);
        final socket = _Socket();
        session.debugAttachSocketForTest(socket);
        final subscription = session.subscribe(
          const NostrFilter(kinds: [0, 10100, 30177, 39002]),
          (_) {},
        );
        final subId = socket.messages.single[1] as String;
        session.debugHandleMessage(['EOSE', subId]);
        final unsubscribe = await subscription;
        addTearDown(unsubscribe);
        final pending = readSelectedMentionAuthorization(
          session,
          {agent.public},
          viewer: viewer.public,
          channelId: 'room',
          isCurrent: () => true,
        );
        await oldQuery.future;
        if (mode == 'retired session' || mode == 'replacement socket') {
          final retained = session.evidenceClock.retain(0, agent.public, null);
          if (mode == 'retired session') {
            container.invalidate(relaySessionProvider);
            container.read(relaySessionProvider);
          } else {
            session.debugSupersedeConnection();
          }
          expect(retained(), isFalse);
          release.complete();
          await expectLater(pending, throwsA(isA<StateError>()));
          final clock = session.evidenceClock;
          expect(
            clock.snapshot(30177, viewer.public, agent.public, [])(),
            isTrue,
          );
          return;
        }
        final newer = switch (mode) {
          'policy' => policy('nobody', 200),
          'runtime' => signed(agent, 10100, {
            'respond_to': 'nobody',
          }, time: 200),
          'missing profile' => profile(agent, [
            authTag(viewer, agent.public),
          ], createdAt: 200),
          'roster' => roster(200),
          _ => profile(other, [], createdAt: 200),
        };
        // Same production socket receipt handler used by discovery/user-cache
        // streams. It runs immediately, before the 16ms UI batch or query finish.
        if (queued) {
          release.complete();
          final evidence = await pending;
          gate.activate(10);
          final publication = withRelayPublicationGuard(
            () {
              if (!evidence.values.every((e) => e.isCurrent())) {
                throw StateError('observed evidence changed before enqueue');
              }
            },
            () => SignedEventRelay(session: session, nsec: viewer.nsec).submit(
              kind: 9000,
              content: '',
              tags: [
                ['h', 'room'],
                ['p', agent.public],
              ],
            ),
          );
          session.debugHandleMessage(['EVENT', subId, newer.toJson()]);
          gate.reset();
          await expectLater(publication, throwsA(isA<StateError>()));
          expect(socket.messages.where((p) => p.first == 'EVENT'), isEmpty);
          session.debugFlushEventBuffer();
          return;
        }
        session.debugHandleMessage(['EVENT', subId, newer.toJson()]);
        release.complete();
        if (mode == 'unrelated') {
          expect((await pending).keys, {agent.public});
        } else {
          await expectLater(pending, throwsA(isA<StateError>()));
        }
      },
    );
  }
}

class _Config extends RelayConfigNotifier {
  _Config(this.key);
  final String key;
  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: key);
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
  final messages = <List<dynamic>>[];
  @override
  void send(List<dynamic> payload) => messages.add(payload);
}

class _Auth extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.unauthenticated);
}
