import 'dart:convert';
import 'package:buzz/shared/auth/enterprise_identity.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final config = jsonEncode({
    'signerUrl': 'https://signer.example/api',
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
      final event = await service.sign(
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
        'https://signer.example/api/v1/buzz/enterprise-signer/events/sign',
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
        service.sign(
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
}
