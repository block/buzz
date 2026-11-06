import 'dart:convert';

import 'package:buzz/onboarding_app_entry.dart';
import 'package:buzz/organization_app_entry.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/organization_environment.dart';
import 'package:buzz/shared/hosted/organization_sign_in.dart';
import 'package:buzz/shared/hosted/self_hosted_entry.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import '../../helpers/widget_helpers.dart';

/// Runs as an internal build, for example:
/// `--dart-define=BUZZ_DISTRIBUTION=internal`
/// `--dart-define=BUZZ_BUILDERLAB_URL=https://id.example.org`
/// `--dart-define=BUZZ_BUILDERLAB_RELAYS=buzz.example.org`
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  testWidgets(
    'one Sign in goes to the organization, joins its community, and a later '
    'landing signs in again',
    (tester) async {
      FlutterSecureStorage.setMockInitialValues({});
      final origin = OrganizationEnvironment.origin!;
      final relay = 'wss://${OrganizationEnvironment.relayHosts.first}';
      final browserUrls = <Uri>[];
      final probes = <Community>[];
      final joined = <Community>[];
      final accounts = <BuilderlabAccountClient>[];
      final expiry = DateTime.now()
          .toUtc()
          .add(const Duration(days: 1))
          .toIso8601String();
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        const MethodChannel('buzz/browser_auth'),
        (_) async => null,
      );
      final client = MockClient((request) async {
        expect(request.url.origin, origin.origin);
        final path = request.url.path;
        final Object result;
        if (path.endsWith('/exchange')) {
          result = {'session_credential': 'session', 'expires_at': expiry};
        } else if (path.endsWith('/me')) {
          result = {'subject': 'member', 'expires_at': expiry};
        } else {
          return http.Response('{}', 404);
        }
        return http.Response(
          jsonEncode(result),
          200,
          headers: {'content-type': 'application/json'},
        );
      });

      Widget app() => WidgetHelpers.testable(
        disableAnimations: true,
        overrides: [
          organizationAccountProvider.overrideWith((ref) {
            final account = BuilderlabAccountClient(
              accountOrigin: origin,
              httpClient: client,
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
          organizationEntryProvider.overrideWithValue(
            SelfHostedEntry(probe: (community) async => probes.add(community)),
          ),
          authProvider.overrideWith(() => _Auth(joined)),
          communityStorageProvider.overrideWithValue(CommunityStorage()),
        ],
        child: const OnboardingAppEntry(),
      );
      await tester.pumpWidget(app());
      await tester.pumpAndSettle();

      expect(find.text('Sign in'), findsOneWidget);
      for (final other in [
        'Scan a QR code',
        'or sign in with email',
        'Enter a community link',
        'Create a community',
      ]) {
        expect(find.text(other), findsNothing, reason: other);
      }

      await tester.tap(find.text('Sign in'));
      await tester.pumpAndSettle();

      final login = browserUrls.single;
      expect(login.origin, origin.origin);
      expect(login.path, '/api/goose/v1/auth/login');
      expect(login.queryParameters['product'], 'buzz');
      expect(probes.single.relayUrl, relay);
      expect(joined.single.relayUrl, relay);
      expect(joined.single.nsec, probes.single.nsec);
      expect(accounts.single.account, isNotNull);

      // Signing out returns here; the saved session is ended, not reused.
      await tester.pumpWidget(const SizedBox());
      await tester.pumpWidget(app());
      await tester.pumpAndSettle();
      expect(accounts.last.account, isNull);
      await tester.tap(find.text('Sign in'));
      await tester.pumpAndSettle();
      expect(browserUrls, hasLength(2));
      expect(joined, hasLength(2));
    },
    skip: !OrganizationEnvironment.enabled,
  );

  test('communities on other relays stay stored but hidden', () async {
    FlutterSecureStorage.setMockInitialValues({});
    final storage = CommunityStorage();
    final organization = Community.create(
      name: 'Organization',
      relayUrl: 'wss://${OrganizationEnvironment.relayHosts.first}',
    );
    final other = Community.create(
      name: 'Other',
      relayUrl: 'wss://relay.example.net',
    );
    final everything = CommunityStorage(visible: (_) => true);
    await everything.saveAll([other, organization]);

    expect((await storage.loadAll()).map((item) => item.id), [organization.id]);
    await storage.remove(organization.id);
    expect(await storage.loadAll(), isEmpty);
    expect((await everything.loadAll()).map((item) => item.id), [other.id]);
  }, skip: !OrganizationEnvironment.enabled);
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

class _Auth extends AuthNotifier {
  _Auth(this.joined);
  final List<Community> joined;

  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.unauthenticated);

  @override
  Future<void> authenticateWithCommunity(
    Community community, {
    bool Function()? shouldContinue,
    bool activate = true,
  }) async => joined.add(community);
}
