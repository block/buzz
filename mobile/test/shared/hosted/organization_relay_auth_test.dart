import 'dart:convert';

import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/organization_environment.dart';
import 'package:buzz/shared/hosted/organization_sign_in.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final keys = nostr.Keys(
    '0000000000000000000000000000000000000000000000000000000000000001',
  );
  final organization = Uri.parse('https://block.example.com');
  const relayHost = 'buzz.block.example.com';
  final relay = RelayConfig(baseUrl: 'https://$relayHost', nsec: keys.nsec);
  final expiry = DateTime.now().toUtc().add(const Duration(days: 1));

  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  group('OrganizationEnvironment', () {
    test('origin and relays are configured together as an HTTPS origin', () {
      expect(OrganizationEnvironment.parseOrigin('', ''), isNull);
      expect(
        OrganizationEnvironment.parseOrigin(
          'https://block.example.com/',
          relayHost,
        ),
        organization,
      );
      for (final (url, relays) in [
        ('https://block.example.com', ''),
        ('', relayHost),
        ('http://block.example.com', relayHost),
        ('https://block.example.com/api/goose', relayHost),
        ('https://block.example.com:8443', relayHost),
        ('https://block.example.com?x=1', relayHost),
        ('https://app.builderlab.xyz', relayHost),
      ]) {
        expect(
          () => OrganizationEnvironment.parseOrigin(url, relays),
          throwsStateError,
          reason: '$url $relays',
        );
      }
    });

    test('relays are exact DNS hosts outside hosted communities', () {
      expect(
        OrganizationEnvironment.parseRelayHosts(' Buzz.Block.Example.com ,a.b'),
        [relayHost, 'a.b'],
      );
      for (final relays in [
        'wss://$relayHost',
        '*.block.example.com',
        'localhost',
        'team.communities.buzz.xyz',
      ]) {
        expect(
          () => OrganizationEnvironment.parseRelayHosts(relays),
          throwsStateError,
          reason: relays,
        );
      }
    });

    test('matches only the configured host on the default port', () {
      bool matches(String url) =>
          OrganizationEnvironment.matches(Uri.parse(url), [relayHost]);
      expect(matches('wss://$relayHost'), isTrue);
      expect(matches('https://$relayHost/'), isTrue);
      expect(matches('wss://$relayHost:443'), isTrue);
      expect(matches('wss://team.$relayHost'), isFalse);
      expect(matches('wss://$relayHost:8443'), isFalse);
      expect(matches('wss://$relayHost/path'), isFalse);
      expect(matches('ws://$relayHost'), isFalse);
    });
  });

  group('organization relay assertions', () {
    late List<http.Request> requests;
    late Object? boundIdentity;

    BuilderlabAccountClient organizationClient() => BuilderlabAccountClient(
      accountOrigin: organization,
      httpClient: MockClient((request) async {
        requests.add(request);
        expect(request.url.origin, organization.origin);
        expect(request.headers['X-BB-Session-Credential'], 'org-secret');
        return switch (request.url.path) {
          '/api/goose/v1/auth/me' => http.Response(
            jsonEncode({
              'subject': 'member',
              'expires_at': expiry.toIso8601String(),
            }),
            200,
          ),
          '/api/goose/v1/buzz/nostr-identities/current' => http.Response(
            jsonEncode({'identity': boundIdentity}),
            200,
          ),
          '/api/goose/v1/buzz/identity/assertions' => http.Response(
            jsonEncode({
              'assertion': 'org-token',
              'nostr_pubkey': keys.public,
              'expires_at': DateTime.now().millisecondsSinceEpoch ~/ 1000 + 120,
            }),
            200,
          ),
          _ => http.Response('{}', 404),
        };
      }),
    );

    Future<void> saveSession() => const FlutterSecureStorage().write(
      key: 'buzz_hosted_session_block_example_com_v1',
      value: jsonEncode({
        'origin': organization.toString(),
        'credential': 'org-secret',
        'subject': 'member',
        'expires_at': expiry.toIso8601String(),
      }),
    );

    HostedRelayAuth relayAuth(BuilderlabAccountClient organizationAccount) =>
        HostedRelayAuth(
          BuilderlabAccountClient(
            httpClient: MockClient(
              (request) async => fail('Hosted account must not be used'),
            ),
          ),
          organization: organizationAccount,
          organizationRelay: (relay) =>
              OrganizationEnvironment.matches(relay, [relayHost]),
          enabled: true,
        );

    setUp(() {
      requests = [];
      boundIdentity = null;
    });

    test(
      'signed-in member gets an assertion for the exact relay and its key',
      () async {
        await saveSession();
        final auth = relayAuth(organizationClient());
        expect(auth.protects(relay), isTrue);
        expect(auth.organizationProtects(relay), isTrue);
        expect(await auth.headers(relay), {
          'Nostr-Federated-Identity': 'Bearer org-token',
        });
        final issue = requests.last;
        expect(issue.url.path, '/api/goose/v1/buzz/identity/assertions');
        expect(jsonDecode(issue.body), {
          'relay_url': 'wss://$relayHost',
          'nostr_pubkey': keys.public,
        });
        expect(issue.headers['Authorization'], startsWith('Nostr '));
        // Cached until near expiry.
        await auth.headers(relay);
        expect(
          requests.where((r) => r.url.path.endsWith('/assertions')),
          hasLength(1),
        );
      },
    );

    test('a key bound to the same member is accepted', () async {
      await saveSession();
      boundIdentity = {'pubkey_hex': keys.public};
      final auth = relayAuth(organizationClient());
      expect(await auth.headers(relay), contains('Nostr-Federated-Identity'));
    });

    test('a different bound key is refused before issuance', () async {
      await saveSession();
      boundIdentity = {'pubkey_hex': 'f' * 64};
      final auth = relayAuth(organizationClient());
      await expectLater(auth.headers(relay), throwsA(AccountFailure.denied));
      expect(auth.failure, AccountFailure.denied);
      expect(
        requests.where((r) => r.url.path.endsWith('/assertions')),
        isEmpty,
      );
    });

    test('signed out connects without an assertion and records it', () async {
      final auth = relayAuth(organizationClient());
      expect(await auth.headers(relay), isEmpty);
      expect(auth.failure, AccountFailure.signedOut);
      expect(requests, isEmpty);
    });

    test('other relays never receive the organization assertion', () async {
      await saveSession();
      final auth = relayAuth(organizationClient());
      for (final other in [
        'https://team.$relayHost',
        'https://other.example.com',
      ]) {
        final config = RelayConfig(baseUrl: other, nsec: keys.nsec);
        expect(auth.protects(config), isFalse);
        expect(await auth.headers(config), isEmpty);
      }
      expect(requests, isEmpty);
    });
  });

  test('each origin owner consumes only its own shared callback', () async {
    late Uri hostedLogin;
    late Uri organizationLogin;
    final hosted = BrowserAuth(
      openBrowser: (uri) async {
        hostedLogin = uri;
        return null;
      },
    );
    final org = BrowserAuth(
      origin: organization,
      openBrowser: (uri) async {
        organizationLogin = uri;
        return null;
      },
    );
    final hostedCodes = <BrowserAuthCode>[];
    final orgCodes = <BrowserAuthCode>[];
    final hostedFailures = <BrowserAuthFailure>[];
    final orgFailures = <BrowserAuthFailure>[];
    hosted.codes.listen(hostedCodes.add);
    hosted.failures.listen(hostedFailures.add);
    org.codes.listen(orgCodes.add);
    org.failures.listen(orgFailures.add);

    await org.start({'purpose': 'organization'});
    expect(organizationLogin.origin, organization.origin);
    expect(organizationLogin.path, '/api/goose/v1/auth/login');
    expect(organizationLogin.queryParameters['returnTo'], BrowserAuth.callback);
    final callback = Uri.parse(BrowserAuth.callback).replace(
      queryParameters: {
        'state': organizationLogin.queryParameters['state']!,
        'code': 'org-code',
      },
    );
    // Both owners see every app link; the hosted owner has no attempt.
    await Future.wait([hosted.receive(callback), org.receive(callback)]);
    await Future<void>.delayed(Duration.zero);
    expect(orgCodes.single.code, 'org-code');
    expect(hostedCodes, isEmpty);
    expect(hostedFailures, isEmpty);
    expect(orgFailures, isEmpty);

    // A pending hosted login is untouched by the organization callback.
    await hosted.start({'relay': 'wss://example.communities.buzz.xyz'});
    await org.start({'purpose': 'organization'});
    final second = Uri.parse(BrowserAuth.callback).replace(
      queryParameters: {
        'state': organizationLogin.queryParameters['state']!,
        'code': 'org-code-2',
      },
    );
    await hosted.receive(second);
    await org.receive(second);
    await Future<void>.delayed(Duration.zero);
    expect(hostedFailures, isEmpty);
    expect(orgCodes.last.code, 'org-code-2');
    await hosted.receive(
      Uri.parse(BrowserAuth.callback).replace(
        queryParameters: {
          'state': hostedLogin.queryParameters['state']!,
          'code': 'hosted-code',
        },
      ),
    );
    await Future<void>.delayed(Duration.zero);
    expect(hostedCodes.single.code, 'hosted-code');
    await hosted.dispose();
    await org.dispose();
  });

  test(
    'an expired abandoned attempt ignores another owner\'s callback',
    () async {
      var now = DateTime.utc(2026, 10, 29);
      final hosted = BrowserAuth(
        openBrowser: (_) async => null,
        now: () => now,
      );
      late Uri organizationLogin;
      final org = BrowserAuth(
        origin: organization,
        now: () => now,
        openBrowser: (uri) async {
          organizationLogin = uri;
          return null;
        },
      );
      final hostedFailures = <BrowserAuthFailure>[];
      final orgCodes = <BrowserAuthCode>[];
      hosted.failures.listen(hostedFailures.add);
      org.codes.listen(orgCodes.add);
      await hosted.start({'relay': 'wss://example.communities.buzz.xyz'});
      await hosted.dispose();
      // After a restart the abandoned hosted attempt is still saved, and stale.
      final restarted = BrowserAuth(now: () => now);
      restarted.failures.listen(hostedFailures.add);
      now = now.add(const Duration(minutes: 30));
      await org.start({'purpose': 'organization'});
      final callback = Uri.parse(BrowserAuth.callback).replace(
        queryParameters: {
          'state': organizationLogin.queryParameters['state']!,
          'code': 'org-code',
        },
      );
      await restarted.receive(callback);
      await org.receive(callback);
      await Future<void>.delayed(Duration.zero);
      expect(hostedFailures, isEmpty);
      expect(orgCodes.single.code, 'org-code');
      await restarted.dispose();
      await org.dispose();
    },
  );

  test('completed sign-in stores the session and reconnects once', () async {
    late Uri login;
    var reconnects = 0;
    final account = BuilderlabAccountClient(
      accountOrigin: organization,
      httpClient: MockClient((request) async {
        expect(request.url.origin, organization.origin);
        if (request.url.path.endsWith('/v1/auth/login/exchange')) {
          return http.Response(
            jsonEncode({
              'session_credential': 'org-secret',
              'expires_at': expiry.toIso8601String(),
            }),
            200,
          );
        }
        if (request.url.path.endsWith('/v1/auth/me')) {
          return http.Response(
            jsonEncode({
              'subject': 'member',
              'expires_at': expiry.toIso8601String(),
            }),
            200,
          );
        }
        return http.Response('{}', 404);
      }),
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
      reconnect: () async => reconnects++,
    );
    await signIn.initialize();
    expect(signIn.status, OrganizationSessionStatus.signedOut);
    await signIn.signIn();
    expect(signIn.status, OrganizationSessionStatus.signingIn);
    await signIn.browser.receive(
      Uri.parse(BrowserAuth.callback).replace(
        queryParameters: {
          'state': login.queryParameters['state']!,
          'code': 'org-code',
        },
      ),
    );
    await pumpEventQueue();
    expect(signIn.status, OrganizationSessionStatus.signedIn);
    expect(signIn.failure, isNull);
    expect(reconnects, 1);
    expect(account.account?.subject, 'member');
    signIn.dispose();
  });

  test(
    'a restore after an offline start shows the session signed in',
    () async {
      await const FlutterSecureStorage().write(
        key: 'buzz_hosted_session_block_example_com_v1',
        value: jsonEncode({
          'origin': organization.toString(),
          'credential': 'org-secret',
          'subject': 'member',
          'expires_at': expiry.toIso8601String(),
        }),
      );
      var online = false;
      final account = BuilderlabAccountClient(
        accountOrigin: organization,
        httpClient: MockClient((request) async {
          if (!online) return http.Response('{}', 503);
          expect(request.url.path, '/api/goose/v1/auth/me');
          return http.Response(
            jsonEncode({
              'subject': 'member',
              'expires_at': expiry.toIso8601String(),
            }),
            200,
          );
        }),
      );
      final signIn = OrganizationSignIn(
        account: account,
        browser: BrowserAuth(
          origin: organization,
          openBrowser: (uri) async => null,
        ),
        reconnect: () async {},
      );
      await signIn.initialize();
      expect(signIn.status, OrganizationSessionStatus.signedOut);

      // A relay connection retry restores the session once back online.
      online = true;
      await account.restore();
      expect(signIn.status, OrganizationSessionStatus.signedIn);
      signIn.dispose();
    },
  );
}
