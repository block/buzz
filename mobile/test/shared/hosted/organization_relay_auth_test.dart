import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/organization_environment.dart';
import 'package:buzz/shared/hosted/organization_key_backup.dart';
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
  const otherRelayHost = 'other.block.example.com';
  final relay = RelayConfig(baseUrl: 'https://$relayHost', nsec: keys.nsec);
  final expiry = DateTime.now().toUtc().add(const Duration(days: 1));

  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  test('organization relays match only the configured host', () {
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

  group('organization relay assertions', () {
    late List<http.Request> requests;
    late Object? boundIdentity;
    // Answers the next identity lookups before falling back to [boundIdentity].
    late List<Object?> nextIdentities;
    late int assertionLifetime;
    late String Function(String relayUrl) tokenFor;

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
            jsonEncode({
              'identity': nextIdentities.isEmpty
                  ? boundIdentity
                  : nextIdentities.removeAt(0),
            }),
            200,
          ),
          '/api/goose/v1/buzz/identity/assertions' => http.Response(
            jsonEncode({
              'assertion': tokenFor(jsonDecode(request.body)['relay_url']),
              'nostr_pubkey': keys.public,
              'expires_at':
                  DateTime.now().millisecondsSinceEpoch ~/ 1000 +
                  assertionLifetime,
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

    HostedRelayAuth relayAuth(
      BuilderlabAccountClient organizationAccount, {
      OrganizationKeyBackup? backup,
    }) => HostedRelayAuth(
      BuilderlabAccountClient(
        httpClient: MockClient(
          (request) async => fail('Hosted account must not be used'),
        ),
      ),
      organization: organizationAccount,
      organizationBackup: backup,
      organizationRelay: (relay) =>
          OrganizationEnvironment.matches(relay, [relayHost, otherRelayHost]),
      enabled: true,
    );

    setUp(() {
      requests = [];
      boundIdentity = null;
      nextIdentities = [];
      assertionLifetime = 120;
      tokenFor = (_) => 'org-token';
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

    test('the issuer chooses the lifetime of an unexpired assertion', () async {
      await saveSession();
      for (final (lifetime, accepted) in [
        (300, true),
        (86400, true),
        (0, false),
        (-60, false),
      ]) {
        assertionLifetime = lifetime;
        final auth = relayAuth(organizationClient());
        if (accepted) {
          expect(await auth.headers(relay), {
            'Nostr-Federated-Identity': 'Bearer org-token',
          });
        } else {
          await expectLater(
            auth.headers(relay),
            throwsA(AccountFailure.invalidResponse),
          );
        }
      }
    });

    test('a key bound to the same member is accepted', () async {
      await saveSession();
      boundIdentity = {'pubkey_hex': keys.public};
      final auth = relayAuth(organizationClient());
      expect(await auth.headers(relay), contains('Nostr-Federated-Identity'));
    });

    test('an issued assertion serves cached reads for the relay', () async {
      await saveSession();
      final organizationAccount = organizationClient();
      final auth = relayAuth(organizationAccount);
      expect(auth.cachedHeaders(relay), isNull);
      await auth.headers(relay);
      expect(auth.cachedHeaders(relay), {
        'Nostr-Federated-Identity': 'Bearer org-token',
      });
      // Cached reads never present a signed-out or replaced account's
      // assertion.
      await organizationAccount.clearLocalSession();
      expect(auth.cachedHeaders(relay), isNull);
      await saveSession();
      expect(await organizationAccount.restore(), isNotNull);
      expect(auth.cachedHeaders(relay), isNull);
    });

    test(
      'issuing for another community keeps the first one cached for reads',
      () async {
        await saveSession();
        tokenFor = (relayUrl) => 'token-for-${Uri.parse(relayUrl).host}';
        final other = RelayConfig(
          baseUrl: 'https://$otherRelayHost',
          nsec: keys.nsec,
        );
        final organizationAccount = organizationClient();
        final backup = _RecordingKeyBackup(organizationAccount);
        final auth = relayAuth(organizationAccount, backup: backup);
        await auth.headers(relay);
        boundIdentity = {'pubkey_hex': keys.public};
        // A background task such as a push revocation for a removed community
        // never backs up the key the user is discarding.
        await auth.headers(other, background: true);
        expect(backup.backedUp, [keys.public]);
        // A background task such as a push revocation issues for an inactive
        // community while `relay` stays the active session.
        await auth.headers(other);
        expect(auth.cachedHeaders(relay), {
          'Nostr-Federated-Identity': 'Bearer token-for-$relayHost',
        });
        expect(auth.cachedHeaders(other), {
          'Nostr-Federated-Identity': 'Bearer token-for-$otherRelayHost',
        });
        // A refusal evicts only the assertion that was refused.
        auth.rejected(other, 'Bearer token-for-$otherRelayHost');
        expect(auth.cachedHeaders(other), isNull);
        expect(auth.cachedHeaders(relay), isNotNull);
        // After the active relay's terminal refusal, a failed background
        // issuance for the other community leaves the active recovery intact.
        auth.rejected(relay, 'Bearer token-for-$relayHost', terminal: true);
        boundIdentity = {'pubkey_hex': 'f' * 64};
        expect(await auth.headers(other, background: true), isEmpty);
        expect(auth.failedRelay, relay.baseUrl);
        expect(auth.failure, AccountFailure.denied);
        // The active relay's reconnect, arriving while that background
        // issuance is still in flight, gets its own assertion rather than
        // the other community's failure.
        boundIdentity = {'pubkey_hex': keys.public};
        tokenFor = (relayUrl) => Uri.parse(relayUrl).host == otherRelayHost
            ? ''
            : 'token-for-${Uri.parse(relayUrl).host}';
        final revocation = expectLater(
          auth.headers(other, background: true),
          completion(isEmpty),
        );
        expect(await auth.headers(relay), {
          'Nostr-Federated-Identity': 'Bearer token-for-$relayHost',
        });
        await revocation;
      },
    );

    test(
      'background issuance never binds a key, shares the cache or fails a foreground caller',
      () async {
        await saveSession();
        var issued = 0;
        tokenFor = (_) => 'token-${++issued}';
        final organizationAccount = organizationClient();
        final backup = _RecordingKeyBackup(organizationAccount);
        final auth = relayAuth(organizationAccount, backup: backup);
        int assertions() =>
            requests.where((r) => r.url.path.endsWith('/assertions')).length;

        // An unbound account is never bound by cleanup with a key the user
        // may have discarded. Cleanup goes out without an assertion and the
        // relay decides; an enforcing relay's refusal is retried durably.
        expect(await auth.headers(relay, background: true), isEmpty);
        expect(assertions(), 0);
        expect(auth.failure, isNull);

        // Each background attempt gets its own assertion, so a retry never
        // resends one the relay refused, and nothing enters the shared cache.
        boundIdentity = {'pubkey_hex': keys.public};
        expect(await auth.headers(relay, background: true), {
          'Nostr-Federated-Identity': 'Bearer token-1',
        });
        expect(await auth.headers(relay, background: true), {
          'Nostr-Federated-Identity': 'Bearer token-2',
        });
        expect(auth.cachedHeaders(relay), isNull);
        expect(backup.backedUp, isEmpty);

        // Foreground use issues its own assertion and backs the key up.
        expect(await auth.headers(relay), {
          'Nostr-Federated-Identity': 'Bearer token-3',
        });
        expect(backup.backedUp, [keys.public]);
        // Nor does cleanup reuse the session's cached assertion, which the
        // relay may already refuse.
        expect(await auth.headers(relay, background: true), {
          'Nostr-Federated-Identity': 'Bearer token-4',
        });

        // After a recorded foreground failure, a background denial for the
        // same relay and key is not handed to a foreground caller waiting on
        // it: that caller issues for itself.
        auth.rejected(relay, 'Bearer token-3');
        tokenFor = (_) => '';
        await expectLater(
          auth.headers(relay),
          throwsA(AccountFailure.invalidResponse),
        );
        expect(auth.failure, AccountFailure.invalidResponse);
        tokenFor = (_) => 'token-${++issued}';
        nextIdentities = [
          {'pubkey_hex': 'f' * 64},
        ];
        final revocation = expectLater(
          auth.headers(relay, background: true),
          completion(isEmpty),
        );
        expect(await auth.headers(relay), {
          'Nostr-Federated-Identity': 'Bearer token-5',
        });
        await revocation;
        expect(auth.failure, isNull);
      },
    );

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

    test('URL headers carry the assertion only to the relay origin', () async {
      await saveSession();
      final auth = relayAuth(organizationClient());
      expect(
        await auth.headersForUrl(
          relay,
          Uri.parse('https://$relayHost/media/emoji.png'),
        ),
        {'Nostr-Federated-Identity': 'Bearer org-token'},
      );
      for (final other in [
        'https://other.example.com/media/emoji.png',
        'https://team.$relayHost/media/emoji.png',
        'http://$relayHost:8080/media/emoji.png',
      ]) {
        expect(await auth.headersForUrl(relay, Uri.parse(other)), isEmpty);
      }
    });

    test('after sign-out only a push stop for a bound key gets an assertion '
        'and nothing can sign back in', () async {
      await saveSession();
      final account = organizationClient();
      final auth = relayAuth(account);
      boundIdentity = {'pubkey_hex': keys.public};
      expect(await auth.headers(relay), isNotEmpty);

      await account.clearLocalSession();
      expect(
        await const FlutterSecureStorage().read(
          key: 'buzz_hosted_session_block_example_com_v1',
        ),
        isNull,
      );
      expect(await account.restore(), isNull);
      expect(account.account, isNull);
      requests.clear();
      tokenFor = (_) => 'push-stop-token';
      expect(await auth.headers(relay, background: true), {
        'Nostr-Federated-Identity': 'Bearer push-stop-token',
      });
      expect(requests.last.headers['X-BB-Session-Credential'], 'org-secret');
      expect(await auth.headers(relay), isEmpty);
      expect(account.account, isNull);

      // A key the account never bound gets nothing.
      boundIdentity = null;
      requests.clear();
      expect(await auth.headers(relay, background: true), isEmpty);
      expect(
        requests.map((request) => request.url.path),
        isNot(contains('/api/goose/v1/buzz/identity/assertions')),
      );
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

  test(
    'a restored owner consuming its callback is never failed by a peer',
    () async {
      late Uri organizationLogin;
      final first = BrowserAuth(
        origin: organization,
        openBrowser: (uri) async {
          organizationLogin = uri;
          return null;
        },
      );
      await first.start({'purpose': 'organization'});
      await first.dispose();
      // After a restart neither owner has the attempt in memory.
      final gated = _PausedReadStorage();
      final org = BrowserAuth(origin: organization, storage: gated);
      final hosted = BrowserAuth(openBrowser: (_) async => null);
      final hostedFailures = <BrowserAuthFailure>[];
      final orgCodes = <BrowserAuthCode>[];
      hosted.failures.listen(hostedFailures.add);
      org.codes.listen(orgCodes.add);
      final callback = Uri.parse(BrowserAuth.callback).replace(
        queryParameters: {
          'state': organizationLogin.queryParameters['state']!,
          'code': 'org-code',
        },
      );
      // The hosted owner's ownership check reads the peer attempt, which
      // the organization owner consumes before that read returns.
      final hostedReceive = hosted.receive(callback);
      await gated.paused.future;
      await org.receive(callback);
      gated.resume.complete();
      await hostedReceive;
      await pumpEventQueue();
      expect(orgCodes.single.code, 'org-code');
      expect(hostedFailures, isEmpty);
      await hosted.dispose();
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

/// Pauses the first read, then serves reads after the peer's changes.
class _PausedReadStorage extends FlutterSecureStorage {
  final paused = Completer<void>();
  final resume = Completer<void>();

  @override
  Future<String?> read({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async {
    if (!paused.isCompleted) {
      paused.complete();
      await resume.future;
    }
    return super.read(key: key);
  }
}

class _RecordingKeyBackup extends OrganizationKeyBackup {
  _RecordingKeyBackup(super.account);

  final backedUp = <String>[];

  @override
  Future<void> afterAssertion(String nsec, String pubkey) async {
    backedUp.add(pubkey);
  }
}
