import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';

const _community = 'abcdabcd-eeee-abcd-abcd-abcdefabcdef';
const _maxBytes = 8 * 1024 * 1024;

void main() {
  final keys = nostr.Keys('1'.padLeft(64, '0'));
  Map<String, dynamic> event({
    String coordinate = 'read-state:device',
    int kind = 30078,
    int createdAt = 1800000000,
  }) =>
      jsonDecode(
            nostr.Event.from(
              kind: kind,
              content: 'opaque encrypted data',
              createdAt: createdAt,
              tags: [
                ['d', coordinate],
              ],
              secretKey: keys.secret,
            ).toJson(),
          )
          as Map<String, dynamic>;
  Map<String, dynamic> envelope({List<Object?>? events}) => {
    'read_state_snapshot': 1,
    'complete': true,
    'community_id': _community,
    'pubkey': keys.public,
    'snapshot_id': 'a' * 64,
    'events': events ?? [event()],
  };
  final discovery = jsonEncode({
    'read_state_snapshot': {
      'version': 1,
      'community_id': _community,
      'max_events': 4096,
      'max_event_array_bytes': _maxBytes,
    },
  });

  _Harness harness(http.Client client) => _Harness(keys.nsec, client);
  http.Client responseClient(Object? response) => MockClient(
    (request) async => http.Response(
      request.method == 'GET' ? discovery : jsonEncode(response),
      200,
    ),
  );

  test(
    'discovers host and sends exact unbounded signed snapshot filter',
    () async {
      final requests = <http.Request>[];
      final h = harness(
        MockClient((request) async {
          requests.add(request);
          return http.Response(
            request.method == 'GET' ? discovery : jsonEncode(envelope()),
            200,
          );
        }),
      );
      final events = await h.session.fetchReadStateSnapshot();
      expect(events.single.id, event()['id']);
      expect(requests, hasLength(2));
      expect(requests.first.url.toString(), 'https://relay.example/');
      expect(requests.first.headers['Accept'], 'application/nostr+json');
      expect(requests.every((r) => !r.followRedirects), isTrue);
      final post = requests.last;
      expect(post.url.toString(), 'https://relay.example/query');
      expect(jsonDecode(post.body), [
        {
          'kinds': [30078],
          'authors': [keys.public],
          'read_state_snapshot': 1,
        },
      ]);
      final proof =
          jsonDecode(
                utf8.decode(
                  base64.decode(post.headers['Authorization']!.substring(6)),
                ),
              )
              as Map<String, dynamic>;
      expect(nostr.Event.fromMap(proof).isValid(), isTrue);
      final tags = proof['tags'] as List;
      expect(tags, contains(equals(['u', post.url.toString()])));
      expect(tags, contains(equals(['method', 'POST'])));
      expect(
        tags,
        contains(
          equals([
            'payload',
            SHA256Digest()
                .process(post.bodyBytes)
                .map((b) => b.toRadixString(16).padLeft(2, '0'))
                .join(),
          ]),
        ),
      );
    },
  );

  test(
    'accepts complete empty state and unrelated own kind-30078 coordinates',
    () async {
      expect(
        await harness(
          responseClient(envelope(events: [])),
        ).session.fetchReadStateSnapshot(),
        isEmpty,
      );
      final events = [event(), event(coordinate: 'other-application')];
      expect(
        await harness(
          responseClient(envelope(events: events)),
        ).session.fetchReadStateSnapshot(),
        hasLength(2),
      );
    },
  );

  for (final entry in <String, Object?>{
    'absent': {},
    'wrong version': {
      'read_state_snapshot': {'version': 2, 'community_id': _community},
    },
    'invalid community': {
      'read_state_snapshot': {'version': 1, 'community_id': 'not-uuid'},
    },
    'non object': [],
  }.entries) {
    test('rejects ${entry.key} discovery before POST', () async {
      var calls = 0;
      final h = harness(
        MockClient((_) async {
          calls++;
          return http.Response(jsonEncode(entry.value), 200);
        }),
      );
      await expectLater(
        h.session.fetchReadStateSnapshot(),
        throwsFormatException,
      );
      expect(calls, 1);
    });
  }

  test('permits valid epoch-zero timestamps', () async {
    final h = harness(responseClient(envelope(events: [event(createdAt: 0)])));
    expect((await h.session.fetchReadStateSnapshot()).single.createdAt, 0);
  });

  final invalidEnvelopes = <String, Object? Function()>{
    'ordinary array': () => [],
    'false completeness': () => envelope()..['complete'] = false,
    'missing completeness': () => envelope()..remove('complete'),
    'wrong version': () => envelope()..['read_state_snapshot'] = 2,
    'other identity': () => envelope()..['pubkey'] = 'b' * 64,
    'other community': () =>
        envelope()..['community_id'] = 'fedcfedc-eeee-abcd-abcd-abcdefabcdef',
    'invalid snapshot id': () => envelope()..['snapshot_id'] = 'A' * 64,
    'invalid event': () => envelope(events: [{}]),
    'non object event': () => envelope(events: [null]),
    'wrong kind': () => envelope(events: [event(kind: 1)]),
    'wrong author': () => envelope(events: [event()..['pubkey'] = 'b' * 64]),
    'invalid signature': () => envelope(events: [event()..['sig'] = '0' * 128]),
    'canonical id mismatch': () =>
        envelope(events: [event()..['content'] = 'tampered']),
    'negative timestamp': () =>
        envelope(events: [event()..['created_at'] = -1]),
    'malformed tags': () => envelope(
      events: [
        event()
          ..['tags'] = [
            ['d', 1],
          ],
      ],
    ),
    'duplicate coordinate': () => envelope(events: [event(), event()]),
    'conflicting coordinate': () => envelope(
      events: [
        event(),
        jsonDecode(
          nostr.Event.from(
            kind: 30078,
            content: 'other version',
            createdAt: 1800000001,
            tags: [
              ['d', 'read-state:device'],
            ],
            secretKey: keys.secret,
          ).toJson(),
        ),
      ],
    ),
    'event count overflow': () => envelope(events: List.filled(4097, {})),
    'event array byte overflow': () =>
        envelope(events: [event()..['content'] = 'x' * _maxBytes]),
  };
  for (final entry in invalidEnvelopes.entries) {
    test('rejects ${entry.key} atomically', () async {
      final h = harness(responseClient(entry.value()));
      await expectLater(
        h.session.fetchReadStateSnapshot(),
        throwsFormatException,
      );
    });
  }

  for (final stage in ['GET', 'POST']) {
    test('$stage carries hosted auth alongside snapshot proof', () async {
      final auth = _SnapshotAuth();
      final session = RelaySessionNotifier(
        httpClient: MockClient((request) async {
          if (request.method == stage &&
              request.headers['Nostr-Federated-Identity'] != 'assertion') {
            return http.Response('Hosted auth required', 401);
          }
          if (request.method == 'POST') {
            expect(request.headers['Authorization'], startsWith('Nostr '));
          }
          return http.Response(
            request.method == 'GET' ? discovery : jsonEncode(envelope()),
            200,
          );
        }),
      );
      _container(session, _Config(keys.nsec), auth: auth);
      expect(await session.fetchReadStateSnapshot(), hasLength(1));
    });

    for (final retirement in ['dispose', 'context change', 'timeout']) {
      test('$stage cannot send after $retirement during hosted auth', () async {
        final entered = Completer<void>();
        final release = Completer<void>();
        var authCalls = 0;
        final auth = _SnapshotAuth(
          beforeHeaders: () async {
            authCalls++;
            if (authCalls == (stage == 'GET' ? 1 : 2)) {
              entered.complete();
              await release.future;
            }
          },
        );
        final sent = <String>[];
        final session = RelaySessionNotifier(
          httpClient: MockClient((request) async {
            sent.add(request.method);
            return http.Response(
              request.method == 'GET' ? discovery : jsonEncode(envelope()),
              200,
            );
          }),
        );
        final config = _Config(keys.nsec);
        _container(session, config, auth: auth);
        final result = expectLater(
          session.fetchReadStateSnapshot(
            timeout: retirement == 'timeout'
                ? const Duration(milliseconds: 100)
                : const Duration(seconds: 5),
          ),
          retirement == 'timeout'
              ? throwsA(isA<TimeoutException>())
              : throwsStateError,
        );
        await entered.future;
        switch (retirement) {
          case 'dispose':
            session.debugDispose();
          case 'context change':
            config.change(keys.nsec, 'https://other.example');
            await Future<void>.delayed(Duration.zero);
          case 'timeout':
            await result;
        }
        release.complete();
        await result;
        // Let late auth completion reach the transport after a timeout.
        await Future<void>.delayed(Duration.zero);
        expect(sent, stage == 'GET' ? isEmpty : ['GET']);
      });
    }
    for (final status in [302, 400, 403, 413, 503]) {
      test('$stage $status is failure, never empty success', () async {
        final h = harness(
          MockClient(
            (r) async => http.Response(
              r.method == stage ? '{}' : discovery,
              r.method == stage ? status : 200,
            ),
          ),
        );
        await expectLater(
          h.session.fetchReadStateSnapshot(),
          throwsA(isA<RelayException>()),
        );
      });
    }
    test('$stage malformed UTF-8 fails closed', () async {
      final h = harness(
        MockClient(
          (r) async => r.method == stage
              ? http.Response.bytes([0xff], 200)
              : http.Response(discovery, 200),
        ),
      );
      await expectLater(
        h.session.fetchReadStateSnapshot(),
        throwsFormatException,
      );
    });
    test(
      '$stage stream is canceled at byte bound before end of body',
      () async {
        var canceled = false;
        final stream = StreamController<List<int>>(
          onCancel: () {
            canceled = true;
          },
        );
        final limit = stage == 'GET' ? 64 * 1024 : _maxBytes + 4096;
        final h = harness(
          _StreamClient((r) async {
            if (r.method != stage) {
              return http.StreamedResponse(
                Stream.value(utf8.encode(discovery)),
                200,
              );
            }
            scheduleMicrotask(() {
              stream.add(List.filled(limit, 32));
              stream.add([32]);
            });
            return http.StreamedResponse(stream.stream, 200);
          }),
        );
        await expectLater(
          h.session.fetchReadStateSnapshot(),
          throwsFormatException,
        );
        expect(canceled, isTrue);
        await stream.close();
      },
    );
    test('$stage response cannot escape disposed context', () async {
      final response = Completer<http.Response>();
      final entered = Completer<void>();
      final h = harness(
        MockClient((r) async {
          if (r.method != stage) return http.Response(discovery, 200);
          entered.complete();
          return response.future;
        }),
      );
      final result = expectLater(
        h.session.fetchReadStateSnapshot(),
        throwsStateError,
      );
      await entered.future;
      h.session.debugDispose();
      response.complete(
        http.Response(stage == 'GET' ? discovery : jsonEncode(envelope()), 200),
      );
      await result;
    });
  }

  test(
    'identity/community change during discovery forbids subsequent POST',
    () async {
      final pending = Completer<http.Response>();
      var calls = 0;
      final h = harness(
        MockClient((_) {
          calls++;
          return pending.future;
        }),
      );
      final result = expectLater(
        h.session.fetchReadStateSnapshot(),
        throwsStateError,
      );
      await Future<void>.delayed(Duration.zero);
      h.config.change(keys.nsec, 'https://other.example');
      pending.complete(http.Response(discovery, 200));
      await result;
      expect(calls, 1);
    },
  );

  test(
    'whole response timeout cancels stalled body and rotates owned client',
    () async {
      var canceled = false;
      final stalled = StreamController<List<int>>(
        onCancel: () {
          canceled = true;
        },
      );
      var created = 0;
      final clients = <_StreamClient>[];
      final config = _Config(keys.nsec);
      final session = RelaySessionNotifier(
        httpClientFactory: () {
          final index = created++;
          final client = _StreamClient(
            (r) async => http.StreamedResponse(
              index == 0
                  ? stalled.stream
                  : Stream.value(
                      utf8.encode(
                        r.method == 'GET'
                            ? discovery
                            : jsonEncode(envelope(events: [])),
                      ),
                    ),
              200,
            ),
          );
          clients.add(client);
          return client;
        },
      );
      _container(session, config);
      await expectLater(
        session.fetchReadStateSnapshot(
          timeout: const Duration(milliseconds: 20),
        ),
        throwsA(isA<TimeoutException>()),
      );
      expect(canceled, isTrue);
      expect(clients.first.closed, isTrue);
      expect(await session.fetchReadStateSnapshot(), isEmpty);
      expect(created, 2);
      await stalled.close();
    },
  );
}

class _Harness {
  final RelaySessionNotifier session;
  final _Config config;
  _Harness(String nsec, http.Client client)
    : session = RelaySessionNotifier(httpClient: client),
      config = _Config(nsec) {
    _container(session, config);
  }
}

void _container(
  RelaySessionNotifier session,
  _Config config, {
  HostedRelayAuth? auth,
}) {
  final container = ProviderContainer(
    overrides: [
      authProvider.overrideWith(_PendingAuth.new),
      if (auth != null) hostedRelayAuthProvider.overrideWithValue(auth),
      relayConfigProvider.overrideWith(() => config),
      relaySessionProvider.overrideWith(() => session),
    ],
  );
  addTearDown(container.dispose);
  container.read(relaySessionProvider);
}

class _Config extends RelayConfigNotifier {
  final String nsec;
  _Config(this.nsec);
  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example/base', nsec: nsec);
  void change(String nsec, String origin) =>
      state = RelayConfig(baseUrl: origin, nsec: nsec);
}

class _PendingAuth extends AuthNotifier {
  @override
  Future<AuthState> build() => Completer<AuthState>().future;
}

class _StreamClient extends http.BaseClient {
  final Future<http.StreamedResponse> Function(http.BaseRequest) handler;
  bool closed = false;
  _StreamClient(this.handler);
  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) =>
      handler(request);
  @override
  void close() {
    closed = true;
  }
}

class _SnapshotAuth extends HostedRelayAuth {
  _SnapshotAuth({this.beforeHeaders}) : super(BuilderlabAccountClient()) {
    addTearDown(() {
      dispose();
      account.dispose();
    });
  }
  final Future<void> Function()? beforeHeaders;

  @override
  bool protects(RelayConfig config) => true;

  @override
  Future<Map<String, String>> headers(RelayConfig config) async {
    await beforeHeaders?.call();
    return {'Nostr-Federated-Identity': 'assertion'};
  }
}
