import 'dart:convert';

import 'package:buzz/features/onboarding/self_hosted_entry_sheet.dart';
import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/onboarding_app_entry.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/device_identity_source.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/organization_environment.dart';
import 'package:buzz/shared/hosted/organization_sign_in.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../helpers/widget_helpers.dart';

/// Runs as an internal build, for example:
/// `--dart-define=BUZZ_DISTRIBUTION=internal`
/// `--dart-define=BUZZ_BUILDERLAB_URL=https://id.example.org`
/// `--dart-define=BUZZ_BUILDERLAB_RELAYS=buzz.example.org`
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final skip = !OrganizationEnvironment.enabled;

  testWidgets(
    'one Sign in joins the organization; after signing out, Sign in reuses '
    'the same identity from this device',
    (tester) async {
      FlutterSecureStorage.setMockInitialValues({});
      final tenant = _Tenant();
      final harness = _Harness(tester, tenant);
      await harness.open();

      expect(find.text('Sign in'), findsOneWidget);
      for (final other in [
        'Scan a QR code',
        'or sign in with email',
        'Enter a community link',
        'Create a community',
      ]) {
        expect(find.text(other), findsNothing, reason: other);
      }

      // The first join is cancelled after its probe bound the new key. The
      // retry must reuse that key, which exists only in the pending slot.
      _Auth.dropNext = true;
      await harness.signIn();
      expect(await harness.saved(), isEmpty);
      final cancelled = harness.probes.single.community;
      expect(tenant.bound, cancelled.pubkey);
      await harness.signIn();

      final login = harness.browserUrls.single;
      expect(login.origin, OrganizationEnvironment.origin!.origin);
      expect(login.path, '/api/goose/v1/auth/login');
      expect(login.queryParameters['product'], 'buzz');
      final first = await harness.joinedCommunity();
      expect(first.relayUrl, harness.relay);
      expect(first.pubkey, cancelled.pubkey);
      expect(tenant.recoveries, 0);
      // The probe carried the session's assertion, which bound the new key.
      expect(
        harness.probes.last.headers['Nostr-Federated-Identity'],
        'Bearer assertion-${first.pubkey}',
      );
      expect(tenant.bound, first.pubkey);
      expect(harness.accounts.last.account, isNotNull);

      // The app backed the new key up to the organization on its own.
      expect(tenant.backups.keys, [first.pubkey]);
      expect(
        tenant.backups[first.pubkey],
        AccountIdentityStore.keysForNsec(first.nsec!).secret,
      );

      // Sign out, at the bottom of Settings, removes the community; the
      // landing that replaces Home then ends the session. Clearing the
      // notification extension's copy fails, so the session stays live.
      const push = MethodChannel('buzz/push');
      final messenger = tester.binding.defaultBinaryMessenger;
      var extensionFails = true;
      messenger.setMockMethodCallHandler(push, (call) async {
        final args = call.arguments;
        if (extensionFails &&
            args is Map &&
            args['section'] == 'organizationSession' &&
            args['credential'] == null) {
          throw PlatformException(code: 'unavailable');
        }
        return null;
      });
      addTearDown(() => messenger.setMockMethodCallHandler(push, null));
      await harness.open(child: const _SignedInGate());
      await harness.settle();
      await tester.tap(find.text('Home'));
      await tester.pumpAndSettle();
      await tester.scrollUntilVisible(
        find.byKey(const Key('settings-sign-out')),
        200,
      );
      await tester.tap(find.byKey(const Key('settings-sign-out')));
      await tester.pumpAndSettle();
      // Only iOS shares the session with its notification extension.
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      await tester.tap(find.widgetWithText(FilledButton, 'Sign out'));
      await harness.settle();
      expect(await harness.saved(), isEmpty);
      // The key stays on this device: sign-in must not depend on the backup,
      // which a device without HPKE sealing never uploads.
      tenant.backups.clear();
      // Signing out closes Settings, uncovering the Sign in landing.
      expect(find.byType(SettingsPage), findsNothing);
      expect(find.text('Sign in'), findsOneWidget);
      expect(harness.accounts.last.account, isNotNull);
      extensionFails = false;

      // Sign in must go through single sign-on again rather than reuse the
      // credential that sign-out was retiring.
      await harness.signIn();
      debugDefaultTargetPlatformOverride = null;
      expect(harness.browserUrls, hasLength(2));
      final second = await harness.joinedCommunity();
      expect(second.pubkey, first.pubkey);
      expect(tenant.recoveries, 0);
    },
    skip: skip,
  );

  // Each row starts signed out with [stored] communities on other relays,
  // which an internal build keeps but hides.
  final rows =
      <
        ({
          String name,
          String? Function(nostr.Keys stored) bound,
          bool backedUp,
          String? expectedError,
          bool picks,
        })
      >[
        (
          name: 'bound to a key on this device uses it without recovery',
          bound: (stored) => stored.public,
          backedUp: false,
          expectedError: null,
          picks: false,
        ),
        (
          name: 'bound to a key only in the organization backup recovers it',
          bound: (_) => _otherKey.public,
          backedUp: true,
          expectedError: null,
          picks: false,
        ),
        (
          name: 'bound to a key with no backup says it cannot restore',
          bound: (_) => _otherKey.public,
          backedUp: false,
          expectedError:
              'Couldn’t restore your organization identity on this device. '
              'Sign in on a device that has it, then try again.',
          picks: false,
        ),
        (
          name: 'unbound with saved keys offers them, then joins',
          bound: (_) => null,
          backedUp: false,
          expectedError: null,
          picks: true,
        ),
      ];
  for (final row in rows) {
    testWidgets(row.name, (tester) async {
      FlutterSecureStorage.setMockInitialValues({});
      final stored = nostr.Keys.generate();
      await CommunityStorage(visible: (_) => true).saveAll([
        Community.create(
          name: 'Elsewhere',
          relayUrl: 'wss://relay.example.net',
          pubkey: stored.public,
          nsec: stored.nsec,
        ),
      ]);
      final tenant = _Tenant()..bound = row.bound(stored);
      if (row.backedUp) tenant.backups[_otherKey.public] = _otherKey.secret;
      final harness = _Harness(tester, tenant);
      await harness.open();
      await harness.signIn();

      if (row.picks) {
        await tester.tap(find.bySemanticsLabel(RegExp('^Use ')));
        await tester.pumpAndSettle();
      }
      if (row.expectedError != null) {
        expect(find.text(row.expectedError!), findsOneWidget);
        expect(harness.probes, isEmpty);
        expect(await harness.saved(), isEmpty);
        return;
      }
      final joined = await harness.joinedCommunity();
      final expected = tenant.bound ?? stored.public;
      expect(joined.pubkey, expected);
      expect(tenant.bound, expected);
      expect(
        harness.probes.single.headers['Nostr-Federated-Identity'],
        'Bearer assertion-$expected',
      );
      expect(tenant.recoveries, row.backedUp ? 1 : 0);
    }, skip: skip);
  }

  test('communities on other relays stay stored but hidden and are never '
      'queried', () async {
    FlutterSecureStorage.setMockInitialValues({});
    final storage = CommunityStorage();
    final keys = nostr.Keys.generate();
    final organization = Community.create(
      name: 'Organization',
      relayUrl: 'wss://${OrganizationEnvironment.relayHosts.first}',
    );
    final other = Community.create(
      name: 'Other',
      relayUrl: 'wss://relay.example.net',
      pubkey: keys.public,
      nsec: keys.nsec,
    );
    final everything = CommunityStorage(visible: (_) => true);
    await everything.saveAll([other, organization]);

    expect((await storage.loadAll()).map((item) => item.id), [organization.id]);

    // A saved identity's profile is never fetched from a hidden relay.
    final queried = <String>[];
    await DeviceIdentitySource().loadProfile(keys.public, (config) async {
      queried.add(config.baseUrl);
      return null;
    });
    expect(queried, isEmpty);

    // Removing the organization's community keeps the hidden one stored.
    await storage.remove(organization.id);
    expect(await storage.loadAll(), isEmpty);
    expect((await everything.loadAll()).map((item) => item.id), [other.id]);
  }, skip: skip);
}

final _otherKey = nostr.Keys.generate();

/// The organization's Builderlab at its HTTP boundary: one member whose
/// account binds the key of its first relay assertion and refuses any other.
/// It holds each backup as the secret key hex that the app sealed for it.
class _Tenant {
  String? bound;
  final backups = <String, String>{};
  String? sealed;
  var enrollments = 0;
  var recoveries = 0;
  final expiry = DateTime.now().toUtc().add(const Duration(days: 1));

  late final client = MockClient((request) async {
    expect(request.url.origin, OrganizationEnvironment.origin!.origin);
    final path = request.url.path.replaceFirst('/api/goose', '');
    final Object result;
    switch (path) {
      case '/v1/auth/login/exchange':
        result = {
          'session_credential': 'session',
          'expires_at': expiry.toIso8601String(),
        };
      case '/v1/auth/me':
        result = {'subject': 'member', 'expires_at': expiry.toIso8601String()};
      case '/v1/buzz/nostr-identities/current':
        result = {
          'identity': bound == null ? null : {'pubkey_hex': bound},
        };
      case '/v1/buzz/identity/assertions':
        final pubkey =
            (jsonDecode(request.body) as Map)['nostr_pubkey'] as String;
        bound ??= pubkey;
        if (bound != pubkey) return http.Response('{}', 403);
        result = {
          'assertion': 'assertion-$pubkey',
          'expires_at': expiry.millisecondsSinceEpoch ~/ 1000,
          'nostr_pubkey': pubkey,
        };
      case '/v1/buzz/nsec-backups/status':
        result = {
          'backup_nostr_pubkeys': backups.keys.toList(),
          'current_nostr_pubkey': bound,
        };
      case '/v1/buzz/nsec-backups/enrollment':
        enrollments++;
        result = {
          'version': 1,
          'suite': {'kem_id': 16, 'kdf_id': 1, 'aead_id': 2},
          'recipient_key_id': 'test-key',
          'service_namespace': 'buzz',
          'owner_id': 'member',
          'backup_id':
              '12345678-1234-1234-1234-${enrollments.toString().padLeft(12, '0')}',
          'nostr_pubkey': bound,
          'recipient_public_key': base64Url
              .encode([4, ...List.filled(64, 1)])
              .replaceAll('=', ''),
        };
      case '/v1/buzz/nsec-backups':
        final envelope = jsonDecode(request.body) as Map;
        backups[envelope['nostr_pubkey'] as String] = sealed!;
        result = {'stored': true, 'backup_id': envelope['backup_id']};
      case '/v1/buzz/nsec-backups/recovery':
        recoveries++;
        result = {
          'backups': [
            for (final MapEntry(key: pubkey, value: secret) in backups.entries)
              {
                'backup_id': 'backup-$pubkey',
                'nostr_pubkey': pubkey,
                'secret_key': base64Url
                    .encode(_hexBytes(secret))
                    .replaceAll('=', ''),
              },
          ],
          'failed_backup_ids': <String>[],
        };
      default:
        return http.Response('{}', 404);
    }
    return http.Response(
      jsonEncode(result),
      200,
      headers: {'content-type': 'application/json'},
    );
  });

  static List<int> _hexBytes(String hex) => [
    for (var i = 0; i < hex.length; i += 2)
      int.parse(hex.substring(i, i + 2), radix: 16),
  ];
}

class _Harness {
  _Harness(this.tester, this.tenant);
  final WidgetTester tester;
  final _Tenant tenant;
  final relay = 'wss://${OrganizationEnvironment.relayHosts.first}';
  final browserUrls = <Uri>[];
  final probes = <({Community community, Map<String, String> headers})>[];
  final accounts = <BuilderlabAccountClient>[];

  Future<List<Community>> saved() => CommunityStorage().loadAll();

  /// The organization's community, durably saved by the join.
  Future<Community> joinedCommunity() async {
    final communities = await saved();
    expect(communities, hasLength(1));
    expect(probes.last.community.nsec, communities.single.nsec);
    return communities.single;
  }

  Future<void> open({Widget child = const OnboardingAppEntry()}) async {
    final origin = OrganizationEnvironment.origin!;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      const MethodChannel('buzz/browser_auth'),
      (_) async => null,
    );
    // Sealing records the plaintext so the fake tenant can later recover it.
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      const MethodChannel('buzz/hosted_hpke'),
      (call) async {
        if (call.method == 'isSupported') return true;
        final plaintext = (call.arguments as Map)['plaintext'] as Uint8List;
        tenant.sealed = [
          for (final byte in plaintext) byte.toRadixString(16).padLeft(2, '0'),
        ].join();
        return {
          'enc': Uint8List.fromList([4, ...List.filled(64, 2)]),
          'ciphertext': Uint8List(48),
        };
      },
    );
    await tester.pumpWidget(const SizedBox());
    await tester.pumpWidget(
      WidgetHelpers.testable(
        disableAnimations: true,
        overrides: [
          organizationAccountProvider.overrideWith((ref) {
            final account = BuilderlabAccountClient(
              accountOrigin: origin,
              httpClient: tenant.client,
              onSessionChanged: (credential) =>
                  shareOrganizationSessionWithNotifications(
                    origin: origin,
                    credential: credential,
                    relayHosts: OrganizationEnvironment.relayHosts,
                  ),
            );
            accounts.add(account);
            ref.onDispose(account.dispose);
            return account;
          }),
          organizationSignInProvider.overrideWith((ref) {
            final signIn = OrganizationSignIn(
              account: ref.watch(organizationAccountProvider)!,
              browser: _Browser(browserUrls.add, origin: origin),
              reconnect: () async {},
            );
            ref.onDispose(signIn.dispose);
            signIn.initialize();
            return signIn;
          }),
          relayProbeProvider.overrideWithValue(
            (community, headers) async =>
                probes.add((community: community, headers: headers)),
          ),
          authProvider.overrideWith(_Auth.new),
          communityStorageProvider.overrideWithValue(CommunityStorage()),
        ],
        child: child,
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> signIn() async {
    await tester.tap(find.text('Sign in'));
    await tester.pumpAndSettle();
    await settle();
  }

  /// Secure storage and HTTP complete outside the fake clock.
  Future<void> settle() async {
    for (var i = 0; i < 5; i++) {
      await tester.runAsync(() => Future<void>.delayed(Duration.zero));
      await tester.pumpAndSettle();
    }
  }
}

/// Home while the organization's community is saved, else the landing, as
/// the app's root does.
class _SignedInGate extends ConsumerWidget {
  const _SignedInGate();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // As HostedAppGate does, keep the organization session alive under Home.
    ref.watch(organizationSignInProvider);
    return ref
        .watch(activeCommunityProvider)
        .when(
          data: (community) => community == null
              ? const OnboardingAppEntry()
              : TextButton(
                  onPressed: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) => SettingsPage(
                        profileHeader: const SizedBox.shrink(),
                        identityRecoveryPageBuilder: (_) =>
                            const SizedBox.shrink(),
                      ),
                    ),
                  ),
                  child: const Text('Home'),
                ),
          loading: () => const SizedBox.shrink(),
          error: (_, _) => const SizedBox.shrink(),
        );
  }
}

class _Browser extends BrowserAuth {
  _Browser(void Function(Uri) onStart, {super.origin})
    : super(
        openBrowser: (uri) async {
          onStart(uri);
          return Uri.parse(BrowserAuth.callback)
              .replace(
                queryParameters: {
                  'state': uri.queryParameters['state']!,
                  'code': 'synthetic-code',
                },
              )
              .toString();
        },
      );
  @override
  Future<void> initialize() async {}
}

/// Admission at the relay boundary: saves the community as the real
/// notifier does once the relay accepts it.
class _Auth extends AuthNotifier {
  /// The next admission returns without saving, as a cancelled one does.
  static var dropNext = false;

  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.unauthenticated);

  @override
  Future<void> authenticateWithCommunity(
    Community community, {
    bool Function()? shouldContinue,
    bool activate = true,
  }) async {
    if (dropNext) {
      dropNext = false;
      return;
    }
    final storage = CommunityStorage();
    await storage.saveAll([...await storage.loadAll(), community]);
  }
}
