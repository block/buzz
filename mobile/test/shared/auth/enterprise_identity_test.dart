import 'dart:convert';
import 'dart:async';
import 'package:buzz/shared/auth/event_signer.dart';
import 'package:buzz/shared/relay/media_auth.dart';
import 'package:buzz/shared/auth/enterprise_identity.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final config = jsonEncode({
    'signerUrl': 'https://signer.example/cash-app/goose/',
    'issuer': 'https://login.example/',
    'clientId': 'native',
    'audience': 'signer',
    'organization': 'org',
    'connection': 'okta',
    'redirectUri': 'buzz://enterprise-login',
  });
  final keys = nostr.Keys.generate();
  final identity = {
    'pubkey': keys.public,
    'relayHttpUrl': 'https://buzz.example',
    'relayWsUrl': 'wss://buzz.example',
  };
  Map<String, dynamic> tokens(int expiresAt) => {
    'access_token': 'access',
    'refresh_token': 'refresh',
    'token_type': 'Bearer',
    'expires_in': 120,
    'expiresAt': expiresAt,
  };
  void seed({required int expiresAt}) {
    FlutterSecureStorage.setMockInitialValues({
      'buzz.enterprise.session': jsonEncode({
        'config': config,
        'identity': identity,
        'tokens': tokens(expiresAt),
      }),
    });
  }

  test(
    'managed media requests satisfy backend policy and refresh at 110 seconds',
    () async {
      var clock = DateTime.now();
      seed(expiresAt: clock.millisecondsSinceEpoch ~/ 1000 + 300);
      final requests = <http.Request>[];
      final expirations = <int>[];
      final owner = EnterpriseIdentity(
        config: config,
        client: MockClient((request) async {
          requests.add(request);
          expect(request.url.path, '/cash-app/goose/v1/buzz/identity/sign');
          expect(request.followRedirects, false);
          final body = jsonDecode(request.body) as Map<String, dynamic>;
          expect(body['purpose'], 'media-read');
          final event = body['event'] as Map<String, dynamic>;
          final tags = (event['tags'] as List)
              .map((t) => List<String>.from(t as List))
              .toList();
          final expiry = int.parse(
            tags.singleWhere((t) => t.first == 'expiration')[1],
          );
          final now = clock.millisecondsSinceEpoch ~/ 1000;
          // IdentitySigningPolicy.kt (703f0219), media-read: expiry in
          // (now + 1)..(now + 300), exact server, kind/tags and fresh timestamp.
          if (expiry < now + 1 || expiry > now + 300) {
            return http.Response('backend media proof lifetime denied', 403);
          }
          expect(event['kind'], 24242);
          expect(tags.length, 3);
          expect(tags, contains(equals(['t', 'get'])));
          expect(tags, contains(equals(['server', 'buzz.example'])));
          expect(event['created_at'], inInclusiveRange(now - 60, now + 30));
          expect(expiry, now + 120);
          expirations.add(expiry);
          final signed = nostr.Event.from(
            kind: event['kind'] as int,
            content: event['content'] as String,
            tags: tags,
            createdAt: event['created_at'] as int,
            secretKey: nostr.Nip19.decode(payload: keys.nsec).data,
            verify: false,
          );
          return http.Response(jsonEncode({'event': signed.toMap()}), 200);
        }),
      );
      await owner.restore();
      final media = MediaGetAuthService(
        baseUrl: 'https://buzz.example',
        nsec: null,
        signer: owner.captureSigner(),
        now: () => clock,
      );
      var mediaRequests = 0;
      final transport = MockClient((request) async {
        mediaRequests++;
        expect(request.followRedirects, false);
        expect(request.headers['Authorization'], startsWith('Nostr '));
        return http.Response('blob', 200);
      });
      final first = await media.headersFor('https://buzz.example/media/blob');
      clock = clock.add(const Duration(seconds: 109));
      expect(
        await media.headersFor('https://buzz.example/media/other'),
        same(first),
      );
      expect(requests.length, 1);
      clock = clock.add(const Duration(seconds: 1));
      await media.get(transport, 'https://buzz.example/media/blob');
      expect(requests.length, 2);
      expect(expirations[1] - expirations[0], 110);
      expect(mediaRequests, 1);
      await owner.logout();
      await expectLater(
        media.get(transport, 'https://buzz.example/media/blob'),
        throwsA(isA<StateError>()),
      );
      expect(mediaRequests, 1);
      expect(requests.length, 2);
    },
  );

  test(
    'restored corporate identity sends only bearer and verifies exact signed event',
    () async {
      seed(expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 120);
      final requests = <http.Request>[];
      final service = EnterpriseIdentity(
        config: config,
        client: MockClient((request) async {
          requests.add(request);
          expect(request.followRedirects, false);
          expect(request.headers['Authorization'], 'Bearer access');
          expect(request.headers.containsKey('X-BB-Session-Credential'), false);
          final body = jsonDecode(request.body) as Map<String, dynamic>;
          final template = body['event'] as Map<String, dynamic>;
          expect(template.containsKey('pubkey'), false);
          final signed = nostr.Event.from(
            kind: template['kind'] as int,
            content: template['content'] as String,
            tags: (template['tags'] as List)
                .map((t) => List<String>.from(t as List))
                .toList(),
            createdAt: template['created_at'] as int,
            secretKey: nostr.Nip19.decode(payload: keys.nsec).data,
            verify: false,
          );
          return http.Response(jsonEncode({'event': signed.toMap()}), 200);
        }),
      );
      await service.restore();
      final event = await signEvent(
        signer: service.captureSigner(),
        kind: 9,
        content: 'hello',
        tags: [
          ['h', 'channel'],
        ],
        createdAt: 1000,
      );
      expect(event.pubkey, keys.public);
      expect(event.createdAt, 1000);
      expect(
        requests.single.url.toString(),
        'https://signer.example/cash-app/goose/v1/buzz/identity/sign',
      );
    },
  );
  test('forged signatures or changed templates fail closed', () async {
    for (final forged in [true, false]) {
      seed(expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 120);
      final service = EnterpriseIdentity(
        config: config,
        client: MockClient((request) async {
          final signed = nostr.Event.from(
            kind: 9,
            content: forged ? 'hello' : 'changed',
            tags: [
              ['h', 'channel'],
            ],
            createdAt: 1000,
            secretKey: nostr.Nip19.decode(payload: keys.nsec).data,
            verify: false,
          ).toMap();
          if (forged) signed['sig'] = '0' * 128;
          return http.Response(jsonEncode({'event': signed}), 200);
        }),
      );
      await service.restore();
      await expectLater(
        signEvent(
          signer: service.captureSigner(),
          kind: 9,
          content: 'hello',
          tags: [
            ['h', 'channel'],
          ],
          createdAt: 1000,
        ),
        throwsA(anything),
      );
    }
  });
  test(
    'refresh is single-flight and rotated credential is persisted before use',
    () async {
      seed(expiresAt: 0);
      var refreshes = 0;
      final service = EnterpriseIdentity(
        config: config,
        client: MockClient((request) async {
          if (request.url.path == '/oauth/token') {
            refreshes++;
            expect(jsonDecode(request.body)['refresh_token'], 'refresh');
            return http.Response(
              jsonEncode({
                'access_token': 'new-access',
                'refresh_token': 'rotated',
                'token_type': 'Bearer',
                'expires_in': 120,
              }),
              200,
            );
          }
          expect(request.headers['Authorization'], 'Bearer new-access');
          return http.Response(jsonEncode(identity), 200);
        }),
      );
      await service.restore();
      expect(
        await Future.wait([service.accessToken(), service.accessToken()]),
        ['new-access', 'new-access'],
      );
      expect(refreshes, 1);
      final saved = jsonDecode(
        (await const FlutterSecureStorage().read(
          key: 'buzz.enterprise.session',
        ))!,
      );
      expect(saved['tokens']['refresh_token'], 'rotated');
    },
  );
  test(
    'refresh cannot switch account and denial never uses local keys',
    () async {
      seed(expiresAt: 0);
      final service = EnterpriseIdentity(
        config: config,
        client: MockClient((request) async {
          if (request.url.path == '/oauth/token') {
            return http.Response(
              jsonEncode({
                'access_token': 'new-access',
                'refresh_token': 'rotated',
                'token_type': 'Bearer',
                'expires_in': 120,
              }),
              200,
            );
          }
          return http.Response(
            jsonEncode({...identity, 'pubkey': nostr.Keys.generate().public}),
            200,
          );
        }),
      );
      await expectLater(service.restore(), throwsStateError);
      expect(service.authenticated, false);
      await expectLater(service.accessToken(), throwsStateError);
    },
  );
  test(
    'credentials cannot be restored by a different release-selected issuer',
    () async {
      seed(expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 120);
      final service = EnterpriseIdentity(
        config: config.replaceAll('login.example', 'other.example'),
        client: MockClient((_) async => throw StateError('must not transmit')),
      );
      await expectLater(service.restore(), throwsStateError);
    },
  );
  test(
    'logout revokes a captured signer even when next login is the same pubkey',
    () async {
      seed(expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 120);
      final owner = EnterpriseIdentity(
        config: config,
        client: MockClient((_) async => throw StateError('must not send')),
      );
      await owner.restore();
      final signer = owner.captureSigner();
      await owner.logout();
      seed(expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 120);
      await owner.restore();
      expect(owner.pubkey, signer.publicKey);
      expect(() => signer.checkCurrent(), throwsStateError);
      expect(
        () => signEvent(signer: signer, kind: 9, content: '', tags: []),
        throwsStateError,
      );
    },
  );

  test(
    'logout during sign discards a valid response from the old login',
    () async {
      seed(expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 120);
      final entered = Completer<void>();
      final response = Completer<http.Response>();
      final owner = EnterpriseIdentity(
        config: config,
        client: MockClient((_) {
          entered.complete();
          return response.future;
        }),
      );
      await owner.restore();
      final signing = signEvent(
        signer: owner.captureSigner(),
        kind: 9,
        content: 'hello',
        tags: [],
        createdAt: 1000,
      );
      final rejected = expectLater(signing, throwsStateError);
      await entered.future;
      await owner.logout();
      final event = nostr.Event.from(
        kind: 9,
        content: 'hello',
        tags: [],
        createdAt: 1000,
        secretKey: keys.secret,
      );
      response.complete(
        http.Response(jsonEncode({'event': event.toMap()}), 200),
      );
      await rejected;
    },
  );

  test(
    'logout during refresh prevents ensure and does not recreate secure storage',
    () async {
      seed(expiresAt: 0);
      final entered = Completer<void>();
      final response = Completer<http.Response>();
      final urls = <String>[];
      final owner = EnterpriseIdentity(
        config: config,
        client: MockClient((request) {
          urls.add(request.url.path);
          entered.complete();
          return response.future;
        }),
      );
      final restoring = expectLater(owner.restore(), throwsStateError);
      await entered.future;
      await owner.logout();
      response.complete(http.Response(jsonEncode(tokens(100)), 200));
      await restoring;
      expect(urls, ['/oauth/token']);
      expect(owner.authenticated, false);
      expect(
        await const FlutterSecureStorage().read(key: 'buzz.enterprise.session'),
        isNull,
      );
    },
  );

  test(
    'corporate media proof is attached only to the exact HTTPS origin',
    () async {
      seed(expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 120);
      var requests = 0;
      final owner = EnterpriseIdentity(
        config: config,
        client: MockClient((request) async {
          requests++;
          final template = jsonDecode(request.body)['event'];
          final signed = nostr.Event.from(
            kind: template['kind'],
            content: template['content'],
            tags: (template['tags'] as List)
                .map((t) => List<String>.from(t))
                .toList(),
            createdAt: template['created_at'],
            secretKey: keys.secret,
          );
          return http.Response(jsonEncode({'event': signed.toMap()}), 200);
        }),
      );
      await owner.restore();
      final auth = MediaGetAuthService(
        baseUrl: owner.relayUrl!,
        nsec: null,
        signer: owner.captureSigner(),
      );
      for (final url in [
        'http://buzz.example/media/a',
        'http://buzz.example:443/media/a',
        'https://buzz.example:80/media/a',
        'https://user@buzz.example/media/a',
        'https://@buzz.example/media/a',
        'https://buzz.example:444/media/a',
        'https://other.example/media/a',
        'https://buzz.example.evil/media/a',
        'https://buzz.example/not-media/a',
      ]) {
        expect(await auth.headersFor(url), isEmpty, reason: url);
      }
      expect(requests, 0);
      final proof = await auth.headersFor('https://buzz.example/media/a');
      expect(proof['Authorization'], startsWith('Nostr '));
      expect(
        await auth.headersFor('https://buzz.example:443/media/b'),
        same(proof),
      );
      expect(requests, 1);
      final fetched = await auth.get(
        MockClient((request) async {
          expect(request.followRedirects, false);
          expect(request.headers['Authorization'], proof['Authorization']);
          return http.Response(
            '',
            302,
            headers: {'location': 'https://evil.example/media/a'},
          );
        }),
        'https://buzz.example/media/a',
      );
      expect(fetched.statusCode, 302);
      auth.dispose();
      await expectLater(
        auth.headersFor('https://buzz.example/media/a'),
        throwsStateError,
      );
    },
  );

  test(
    'login uses PKCE, exact callback, scope intent and server-selected identity',
    () async {
      FlutterSecureStorage.setMockInitialValues({});
      final callbacks = StreamController<Uri>();
      final requests = <http.Request>[];
      final owner = EnterpriseIdentity(
        config: config,
        callbacks: callbacks.stream,
        openBrowser: (uri) async {
          expect(
            uri.queryParameters['scope'],
            'openid profile email offline_access buzz:sign',
          );
          expect(uri.queryParameters['code_challenge_method'], 'S256');
          expect(
            uri.queryParameters['redirect_uri'],
            'buzz://enterprise-login',
          );
          expect(uri.queryParameters.containsKey('client_secret'), false);
          final state = uri.queryParameters['state']!;
          callbacks.add(
            Uri.parse('buzz://user@enterprise-login?code=wrong&state=$state'),
          );
          callbacks.add(
            Uri.parse('buzz://enterprise-login?code=correct&state=$state'),
          );
          return true;
        },
        client: MockClient((request) async {
          requests.add(request);
          expect(request.followRedirects, false);
          if (request.url.path == '/oauth/token') {
            expect(jsonDecode(request.body)['code'], 'correct');
            expect(jsonDecode(request.body)['code_verifier'], isNotEmpty);
            return http.Response(jsonEncode(tokens(0)), 200);
          }
          expect(request.url.path, '/cash-app/goose/v1/buzz/identity/ensure');
          expect(jsonDecode(request.body), isEmpty);
          return http.Response(jsonEncode(identity), 200);
        }),
      );
      await owner.login();
      expect(owner.authenticated, true);
      expect(owner.captureSigner().publicKey, keys.public);
      expect(requests.length, 2);
      await callbacks.close();
    },
  );

  test(
    'malformed native config fails before browser, storage or transport',
    () async {
      final valid = jsonDecode(config) as Map<String, dynamic>;
      for (final bad in [
        {...valid, 'redirectUri': 'buzz://enterprise-login/other'},
        {...valid, 'signerUrl': 'http://signer.example/cash-app/goose/'},
        {
          ...valid,
          'signerUrl': 'https://signer.example/v1/buzz/identity/ensure',
        },
        {...valid, 'issuer': 'https://user@login.example/'},
        {...valid, 'clientSecret': 'forbidden'},
        {...valid, 'environment': 'staging'},
        {...valid, 'clientId': ''},
        {...valid, 'signerUrl': 'https://signer.example:443/cash-app/goose/'},
        {...valid, 'issuer': 'https://@login.example/'},
      ]) {
        final owner = EnterpriseIdentity(
          config: jsonEncode(bad),
          openBrowser: (_) async => throw StateError('must not open'),
          client: MockClient((_) async => throw StateError('must not send')),
        );
        await expectLater(owner.restore(), throwsA(anything));
      }
    },
  );
  test(
    'simultaneous expired-token consumers join the active refresh',
    () async {
      seed(expiresAt: 0);
      final entered = Completer<void>();
      final release = Completer<void>();
      var refreshes = 0;
      final owner = EnterpriseIdentity(
        config: config,
        client: MockClient((request) async {
          if (request.url.path == '/oauth/token') {
            refreshes++;
            entered.complete();
            await release.future;
            return http.Response(jsonEncode(tokens(0)), 200);
          }
          return http.Response(jsonEncode(identity), 200);
        }),
      );
      final restoring = owner.restore();
      await entered.future;
      final first = owner.accessToken();
      final second = owner.accessToken();
      release.complete();
      await restoring;
      expect(await Future.wait([first, second]), ['access', 'access']);
      expect(refreshes, 1);
    },
  );

  test(
    'logout cancels a pending browser callback without waiting five minutes',
    () async {
      seed(expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 120);
      final callbacks = StreamController<Uri>();
      final opened = Completer<void>();
      final owner = EnterpriseIdentity(
        config: config,
        callbacks: callbacks.stream,
        openBrowser: (_) async {
          opened.complete();
          return true;
        },
        client: MockClient((_) async => throw StateError('must not send')),
      );
      final login = expectLater(owner.login(), throwsStateError);
      await opened.future;
      await owner.logout();
      await login.timeout(const Duration(seconds: 1));
      expect(
        await const FlutterSecureStorage().read(key: 'buzz.enterprise.session'),
        isNull,
      );
      await callbacks.close();
    },
  );

  test(
    'failed replacement login cannot restore the previous saved account',
    () async {
      seed(expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 120);
      final callbacks = StreamController<Uri>();
      final owner = EnterpriseIdentity(
        config: config,
        callbacks: callbacks.stream,
        openBrowser: (_) async => false,
        client: MockClient((_) async => throw StateError('must not send')),
      );
      await owner.restore();
      final old = owner.captureSigner();
      await expectLater(owner.login(), throwsStateError);
      expect(() => old.checkCurrent(), throwsStateError);
      await owner.restore();
      expect(owner.authenticated, false);
      expect(
        await const FlutterSecureStorage().read(key: 'buzz.enterprise.session'),
        isNull,
      );
      await callbacks.close();
    },
  );

  test(
    'valid signatures cannot rewrite any exact event field or author',
    () async {
      for (final changed in [
        'kind',
        'content',
        'tags',
        'created_at',
        'pubkey',
      ]) {
        seed(expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 120);
        final owner = EnterpriseIdentity(
          config: config,
          client: MockClient((request) async {
            final signed = nostr.Event.from(
              kind: changed == 'kind' ? 10 : 9,
              content: changed == 'content' ? 'changed' : 'exact',
              tags: [
                ['h', changed == 'tags' ? 'other' : 'channel'],
              ],
              createdAt: changed == 'created_at' ? 1001 : 1000,
              secretKey: changed == 'pubkey'
                  ? nostr.Keys.generate().secret
                  : keys.secret,
            );
            return http.Response(jsonEncode({'event': signed.toMap()}), 200);
          }),
        );
        await owner.restore();
        await expectLater(
          signEvent(
            signer: owner.captureSigner(),
            kind: 9,
            content: 'exact',
            tags: [
              ['h', 'channel'],
            ],
            createdAt: 1000,
          ),
          throwsStateError,
          reason: changed,
        );
      }
    },
  );

  test(
    'managed media signing failure propagates, never unsigned fallback',
    () async {
      seed(expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 120);
      final owner = EnterpriseIdentity(
        config: config,
        client: MockClient((_) async => http.Response('', 403)),
      );
      await owner.restore();
      final auth = MediaGetAuthService(
        baseUrl: owner.relayUrl!,
        nsec: keys.nsec,
        signer: owner.captureSigner(),
      );
      await expectLater(
        auth.headersFor('https://buzz.example/media/a'),
        throwsStateError,
      );
    },
  );
}
