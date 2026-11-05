import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/organization_sign_in.dart';
import 'package:flutter/services.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final organization = Uri.parse('https://block.example.com');
  const orgSessionKey = 'buzz_hosted_session_block_example_com_v1';
  final expiry = DateTime.now().toUtc().add(const Duration(days: 1));

  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  http.Response me() => http.Response(
    jsonEncode({'subject': 'member', 'expires_at': expiry.toIso8601String()}),
    200,
  );
  http.Response exchanged() => http.Response(
    jsonEncode({
      'session_credential': 'org-secret',
      'expires_at': expiry.toIso8601String(),
    }),
    200,
  );
  Uri callback(Uri login, String code) =>
      Uri.parse(BrowserAuth.callback).replace(
        queryParameters: {
          'state': login.queryParameters['state']!,
          'code': code,
        },
      );

  test('a cancelled exchange never changes a newer sign-in', () async {
    final logins = <Uri>[];
    final held = [Completer<http.Response>(), Completer<http.Response>()];
    var exchanges = 0;
    http.Response exchangedAs(String credential) => http.Response(
      jsonEncode({
        'session_credential': credential,
        'expires_at': expiry.toIso8601String(),
      }),
      200,
    );
    final signIn = OrganizationSignIn(
      account: BuilderlabAccountClient(
        accountOrigin: organization,
        httpClient: MockClient((request) async {
          if (request.url.path.endsWith('/v1/auth/login/exchange')) {
            final index = exchanges++;
            return index < held.length
                ? held[index].future
                : exchangedAs('org-member-b');
          }
          final credential = request.headers['X-BB-Session-Credential']!;
          return http.Response(
            jsonEncode({
              'subject': credential.substring('org-'.length),
              'expires_at': expiry.toIso8601String(),
            }),
            200,
          );
        }),
      ),
      browser: BrowserAuth(
        origin: organization,
        openBrowser: (uri) async {
          logins.add(uri);
          return null;
        },
      ),
      reconnect: () async {},
    );
    await signIn.initialize();
    await signIn.signIn();
    await signIn.browser.receive(callback(logins.last, 'code-1'));
    await pumpEventQueue();
    expect(exchanges, 1);

    // A cancelled exchange that fails late leaves the newer attempt alone.
    await signIn.cancel();
    await signIn.signIn();
    expect(signIn.status, OrganizationSessionStatus.signingIn);
    held[0].complete(http.Response('{}', 503));
    await pumpEventQueue();
    expect(signIn.status, OrganizationSessionStatus.signingIn);
    expect(signIn.failure, isNull);

    // The newer attempt still owns its browser and can be cancelled.
    await signIn.cancel();
    expect(signIn.status, OrganizationSessionStatus.signedOut);
    await signIn.signIn();

    // A cancelled exchange that succeeds late is never saved, so the newer
    // sign-in as a different member still succeeds.
    await signIn.browser.receive(callback(logins.last, 'code-2'));
    await pumpEventQueue();
    expect(exchanges, 2);
    await signIn.cancel();
    await signIn.signIn();
    await signIn.browser.receive(callback(logins.last, 'code-3'));
    held[1].complete(exchangedAs('org-member-a'));
    await pumpEventQueue();
    expect(signIn.failure, isNull);
    expect(signIn.status, OrganizationSessionStatus.signedIn);
    expect(signIn.account.account?.subject, 'member-b');
    expect(
      jsonDecode((await const FlutterSecureStorage().read(key: orgSessionKey))!)
          as Map,
      containsPair('subject', 'member-b'),
    );
    signIn.dispose();
  });

  test('a second tap while the session clears starts no attempt', () async {
    await const FlutterSecureStorage().write(
      key: orgSessionKey,
      value: 'not json',
    );
    final logins = <Uri>[];
    final browserStorage = _CountingStorage();
    final signIn = OrganizationSignIn(
      account: BuilderlabAccountClient(
        accountOrigin: organization,
        httpClient: MockClient((request) async => me()),
      ),
      browser: BrowserAuth(
        origin: organization,
        storage: browserStorage,
        openBrowser: (uri) async {
          logins.add(uri);
          return null;
        },
      ),
      reconnect: () async {},
    );
    await signIn.initialize();
    expect(signIn.failure, AccountFailure.storage);
    await Future.wait([signIn.signIn(), signIn.signIn()]);
    expect(signIn.status, OrganizationSessionStatus.signingIn);
    expect(browserStorage.attempts, 1);
    signIn.dispose();
  });

  test('a failed intent cleanup still leaves sign-in signed in', () async {
    late Uri login;
    var reconnects = 0;
    final storage = _FailingDeleteStorage('buzz_browser_auth_intent_v1_');
    final signIn = OrganizationSignIn(
      account: BuilderlabAccountClient(
        accountOrigin: organization,
        httpClient: MockClient(
          (request) async =>
              request.url.path.endsWith('/exchange') ? exchanged() : me(),
        ),
      ),
      browser: BrowserAuth(
        origin: organization,
        storage: storage,
        openBrowser: (uri) async {
          login = uri;
          return null;
        },
      ),
      reconnect: () async => reconnects++,
    );
    await signIn.initialize();
    await signIn.signIn();
    await signIn.browser.receive(callback(login, 'org-code'));
    await pumpEventQueue();
    expect(signIn.status, OrganizationSessionStatus.signedIn);
    expect(signIn.failure, BrowserAuthFailure.storage);
    expect(reconnects, 1);
    signIn.dispose();
  });

  test('a failed cancel cleanup still leaves the busy state', () async {
    final storage = _FailingDeleteStorage('buzz_browser_auth_attempt_v1_');
    final signIn = OrganizationSignIn(
      account: BuilderlabAccountClient(
        accountOrigin: organization,
        httpClient: MockClient((request) async => me()),
      ),
      browser: BrowserAuth(
        origin: organization,
        storage: storage,
        openBrowser: (uri) async => null,
      ),
      reconnect: () async {},
    );
    await signIn.initialize();
    await signIn.signIn();
    expect(signIn.status, OrganizationSessionStatus.signingIn);
    await signIn.cancel();
    expect(signIn.status, OrganizationSessionStatus.signedOut);
    expect(signIn.failure, BrowserAuthFailure.storage);
    signIn.dispose();
  });

  test('an unreadable saved session is replaced by signing in', () async {
    await const FlutterSecureStorage().write(
      key: orgSessionKey,
      value: 'not json',
    );
    late Uri login;
    final account = BuilderlabAccountClient(
      accountOrigin: organization,
      httpClient: MockClient(
        (request) async =>
            request.url.path.endsWith('/exchange') ? exchanged() : me(),
      ),
    );
    final signIn = OrganizationSignIn(
      account: account,
      browser: BrowserAuth(
        origin: organization,
        openBrowser: (uri) async {
          login = uri;
          return null;
        },
      ),
      reconnect: () async {},
    );
    await signIn.initialize();
    expect(signIn.status, OrganizationSessionStatus.signedOut);
    expect(signIn.failure, AccountFailure.storage);

    await signIn.signIn();
    await signIn.browser.receive(callback(login, 'org-code'));
    await pumpEventQueue();
    expect(signIn.status, OrganizationSessionStatus.signedIn);
    expect(signIn.failure, isNull);
    expect(account.account?.subject, 'member');
    signIn.dispose();
  });
}

/// Fails deletions of keys with [prefix], as an unavailable Keychain would.
class _FailingDeleteStorage extends FlutterSecureStorage {
  _FailingDeleteStorage(this.prefix);
  final String prefix;

  @override
  Future<void> delete({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) {
    if (key.startsWith(prefix)) {
      throw PlatformException(code: 'keychain', message: 'unavailable');
    }
    return super.delete(key: key);
  }
}

/// Counts browser sign-in attempts as they are saved.
class _CountingStorage extends FlutterSecureStorage {
  int attempts = 0;

  @override
  Future<void> write({
    required String key,
    required String? value,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) {
    if (key.startsWith('buzz_browser_auth_attempt_v1_')) attempts++;
    return super.write(key: key, value: value);
  }
}
