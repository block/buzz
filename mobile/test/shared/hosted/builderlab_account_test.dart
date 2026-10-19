import 'dart:async';
import 'hosted_http_fixture.dart';
import 'dart:convert';

import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final expiry = DateTime.now()
      .toUtc()
      .add(const Duration(days: 1))
      .toIso8601String();
  final verifier = List.filled(43, 'a').join();
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  test(
    'real client exchanges PKCE code, verifies, persists and restores session',
    () async {
      final requests = <http.Request>[];
      final transport = MockClient((request) async {
        requests.add(request);
        expect(request.url.origin, 'https://app.builderlab.xyz');
        expect(request.followRedirects, isFalse);
        expect(request.headers.containsKey('cookie'), isFalse);
        if (request.url.path.endsWith('/exchange')) {
          expect(jsonDecode(request.body), {
            'code': 'one-use',
            'code_verifier': verifier,
          });
          expect(
            request.headers.containsKey('X-BB-Session-Credential'),
            isFalse,
          );
          return http.Response(
            jsonEncode({'session_credential': 'secret', 'expires_at': expiry}),
            200,
          );
        }
        expect(request.headers['X-BB-Session-Credential'], 'secret');
        return http.Response(
          jsonEncode({'subject': 'account-1', 'expires_at': expiry}),
          200,
        );
      });
      final client = BuilderlabAccountClient(httpClient: transport);
      expect(
        (await client.exchange(code: 'one-use', verifier: verifier)).subject,
        'account-1',
      );
      final restored = BuilderlabAccountClient(httpClient: transport);
      expect((await restored.restore())!.subject, 'account-1');
      expect(requests.length, 3);
      await restored.clearLocalSession();
      expect(
        await BuilderlabAccountClient(httpClient: transport).restore(),
        isNull,
      );
    },
  );

  testWidgets('total deadline releases account queue despite trickling body', (
    tester,
  ) async {
    final transport = TrickleClient(
      (request) => http.Response(
        jsonEncode(
          request.url.path.endsWith('/exchange')
              ? {'session_credential': 'secret', 'expires_at': expiry}
              : {'subject': 'account-1', 'expires_at': expiry},
        ),
        200,
      ),
    );
    final client = BuilderlabAccountClient(httpClient: transport);
    addTearDown(client.dispose);
    Object? failure;
    var cleared = false;
    unawaited(
      client
          .exchange(code: 'code', verifier: verifier)
          .then<void>(
            (_) => fail('trickling response must not complete'),
            onError: (Object error) => failure = error,
          ),
    );
    unawaited(client.clearLocalSession().then((_) => cleared = true));
    await tester.pump();
    await tester.pump(const Duration(seconds: 20));
    await tester.pump(const Duration(seconds: 5));
    expect(failure, isNull);
    expect(cleared, isFalse);
    await tester.pump(const Duration(seconds: 5));
    expect(failure, AccountFailure.unavailable);
    expect(transport.aborted, isTrue);
    expect(transport.cancelled, isTrue);
    expect(cleared, isTrue);
    transport.slow = false;
    final retry = client.exchange(code: 'retry', verifier: verifier);
    await tester.pump();
    expect((await retry).subject, 'account-1');
  });

  test('invalid verification cannot persist an exchanged credential', () async {
    final transport = MockClient(
      (request) async => http.Response(
        jsonEncode(
          request.url.path.endsWith('/exchange')
              ? {'session_credential': 'secret', 'expires_at': expiry}
              : {'expires_at': expiry},
        ),
        200,
      ),
    );
    final client = BuilderlabAccountClient(httpClient: transport);
    await expectLater(
      client.exchange(code: 'code', verifier: verifier),
      throwsA(AccountFailure.invalidResponse),
    );
    expect(await client.restore(), isNull);
  });

  test('redirect and denial are not accepted as login success', () async {
    for (final status in [302, 403]) {
      final client = BuilderlabAccountClient(
        httpClient: MockClient(
          (_) async => http.Response('sensitive response', status),
        ),
      );
      await expectLater(
        client.exchange(code: 'code', verifier: verifier),
        throwsA(
          status == 403 ? AccountFailure.denied : AccountFailure.unavailable,
        ),
      );
      expect(await client.restore(), isNull);
    }
  });
  const storageKey = 'buzz_hosted_session_app_builderlab_xyz_v1';
  String record(String subject, String credential) => jsonEncode({
    'origin': 'https://app.builderlab.xyz',
    'credential': credential,
    'subject': subject,
    'expires_at': expiry,
  });
  Future<String?> stored() =>
      const FlutterSecureStorage().read(key: storageKey);

  test('session storage is device-only on iOS', () {
    expect(
      BuilderlabAccountClient.defaultStorage.iOptions.accessibility,
      KeychainAccessibility.first_unlock_this_device,
    );
  });

  test('callback exchange waits for an in-flight startup restore', () async {
    FlutterSecureStorage.setMockInitialValues({
      storageKey: record('account-1', 'old'),
    });
    final restoreVerified = Completer<void>();
    final client = BuilderlabAccountClient(
      httpClient: MockClient((request) async {
        if (request.url.path.endsWith('/exchange')) {
          return http.Response(
            jsonEncode({'session_credential': 'new', 'expires_at': expiry}),
            200,
          );
        }
        if (request.headers['X-BB-Session-Credential'] == 'old') {
          await restoreVerified.future;
        }
        return http.Response(
          jsonEncode({'subject': 'account-1', 'expires_at': expiry}),
          200,
        );
      }),
    );
    final restoring = client.restore();
    final exchanging = client.exchange(code: 'code', verifier: verifier);
    restoreVerified.complete();
    expect((await restoring)!.subject, 'account-1');
    expect((await exchanging).subject, 'account-1');
    expect(jsonDecode((await stored())!)['credential'], 'new');
  });

  test('login for another account keeps the existing session', () async {
    FlutterSecureStorage.setMockInitialValues({
      storageKey: record('account-1', 'old'),
    });
    final client = BuilderlabAccountClient(
      httpClient: MockClient(
        (request) async => http.Response(
          jsonEncode(
            request.url.path.endsWith('/exchange')
                ? {'session_credential': 'new', 'expires_at': expiry}
                : {'subject': 'account-2', 'expires_at': expiry},
          ),
          200,
        ),
      ),
    );
    await expectLater(
      client.exchange(code: 'code', verifier: verifier),
      throwsA(AccountFailure.differentAccount),
    );
    expect(client.account, isNull);
    expect(await stored(), record('account-1', 'old'));
  });

  test('only a rejected credential clears a stored session', () async {
    for (final (status, failure) in [
      (401, null),
      (500, AccountFailure.unavailable),
      (403, AccountFailure.denied),
    ]) {
      FlutterSecureStorage.setMockInitialValues({
        storageKey: record('account-1', 'old'),
      });
      final client = BuilderlabAccountClient(
        httpClient: MockClient((_) async => http.Response('{}', status)),
      );
      if (failure == null) {
        expect(await client.restore(), isNull);
        expect(await stored(), isNull);
      } else {
        await expectLater(client.restore(), throwsA(failure));
        expect(await stored(), record('account-1', 'old'));
      }
    }
  });

  test(
    'unreadable record is a storage failure, not an absent session',
    () async {
      FlutterSecureStorage.setMockInitialValues({storageKey: 'not json'});
      final client = BuilderlabAccountClient(
        httpClient: MockClient((_) async => fail('no request expected')),
      );
      await expectLater(client.restore(), throwsA(AccountFailure.storage));
      await expectLater(
        client.exchange(code: 'code', verifier: verifier),
        throwsA(AccountFailure.storage),
      );
      await client.clearLocalSession();
      expect(await client.restore(), isNull);
    },
  );

  test('rejected identity read clears the session', () async {
    var sessionRevoked = false;
    final client = BuilderlabAccountClient(
      httpClient: MockClient((request) async {
        if (request.url.path.endsWith('/current')) {
          return http.Response('{}', 401);
        }
        if (request.url.path.endsWith('/me') &&
            request.headers['X-BB-Session-Credential'] == 'secret' &&
            sessionRevoked) {
          return http.Response('{}', 401);
        }
        return http.Response(
          jsonEncode(
            request.url.path.endsWith('/exchange')
                ? {'session_credential': 'secret', 'expires_at': expiry}
                : {'subject': 'account-1', 'expires_at': expiry},
          ),
          200,
        );
      }),
    );
    await client.exchange(code: 'code', verifier: verifier);
    // A route-level 401 with a valid session is an authorization failure.
    await expectLater(client.currentIdentity(), throwsA(AccountFailure.denied));
    expect(client.account, isNotNull);
    expect(await stored(), isNotNull);
    sessionRevoked = true;
    await expectLater(
      client.currentIdentity(),
      throwsA(AccountFailure.signedOut),
    );
    expect(client.account, isNull);
    expect(await stored(), isNull);
  });

  test('authenticated post accepts only Buzz account routes', () async {
    final client = BuilderlabAccountClient(
      httpClient: MockClient((_) async => fail('no request expected')),
    );
    for (final path in [
      '/v1/auth/me',
      '/v1/buzz',
      '/v1/buzz/../auth/me',
      '/v1/buzz/nsec-backups?x=1',
      'https://evil.example/v1/buzz/nsec-backups',
      '//evil.example/v1/buzz/nsec-backups',
    ]) {
      expect(() => client.post(path, {}), throwsArgumentError, reason: path);
    }
    await expectLater(
      client.post('/v1/buzz/nsec-backups/enrollment', {}),
      throwsA(AccountFailure.signedOut),
    );
  });
}
