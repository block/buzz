import 'dart:convert';
import 'package:flutter/services.dart';
import 'package:buzz/features/onboarding/organization_sign_in_banner.dart';
import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/organization_sign_in.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:buzz/shared/relay/relay_session.dart';
import 'package:buzz/shared/theme/app_theme.dart';
import 'package:buzz/shared/widgets/buzz_toast.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

class _Config extends RelayConfigNotifier {
  @override
  RelayConfig build() =>
      const RelayConfig(baseUrl: 'https://organization.example');
}

class _Session extends RelaySessionNotifier {
  int retries = 0;
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);
  @override
  Future<void> reconnect() async {
    retries++;
  }
}

void main() {
  setUpAll(() async {
    final font = FontLoader('Inter')
      ..addFont(rootBundle.load('assets/fonts/InterVariable.ttf'));
    await font.load();
  });
  late OrganizationSignIn signIn;
  final logins = <Uri>[];
  late HostedRelayAuth auth;
  late _Session session;
  setUp(() async {
    FlutterSecureStorage.setMockInitialValues({});
    logins.clear();
    final expiry = DateTime.now().toUtc().add(const Duration(days: 1));
    final account = BuilderlabAccountClient(
      accountOrigin: Uri.parse('https://organization.example'),
      httpClient: MockClient(
        (request) async => http.Response(
          jsonEncode({
            if (request.url.path.endsWith('/exchange'))
              'session_credential': 'org-secret'
            else
              'subject': 'member',
            'expires_at': expiry.toIso8601String(),
          }),
          200,
        ),
      ),
    );
    signIn = OrganizationSignIn(
      account: account,
      browser: BrowserAuth(
        origin: account.accountOrigin,
        openBrowser: (uri) async {
          logins.add(uri);
          return null;
        },
      ),
      reconnect: () async {},
    );
    await signIn.initialize();
    auth = HostedRelayAuth(
      account,
      organization: account,
      organizationRelay: (_) => true,
    );
    session = _Session();
    addTearDown(() {
      signIn.dispose();
      auth.dispose();
      account.dispose();
    });
  });
  Future<void> mount(WidgetTester tester, {double scale = 1}) async {
    await tester.binding.setSurfaceSize(const Size(320, 568));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          organizationSignInProvider.overrideWithValue(signIn),
          hostedRelayAuthProvider.overrideWithValue(auth),
          relayConfigProvider.overrideWith(_Config.new),
          relaySessionProvider.overrideWith(() => session),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: MediaQuery(
              data: MediaQueryData(
                size: const Size(320, 568),
                textScaler: TextScaler.linear(scale),
              ),
              child: Stack(
                children: [
                  const OrganizationSignInBanner(),
                  Positioned(
                    bottom: 0,
                    child: Builder(
                      builder: (context) => TextButton(
                        onPressed: () => showBuzzToast(context, 'Saved'),
                        child: const Text('Toast'),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  for (final scale in [1.0, 2.0, 3.0]) {
    testWidgets(
      'sign in, cancel, retry sign-in and successful completion at $scale',
      (tester) async {
        await mount(tester, scale: scale);
        expect(find.text("It looks like you're signed out."), findsOneWidget);
        expectVisibleCard(tester);
        await tester.tap(find.text('Sign in'));
        await tester.pumpAndSettle();
        expect(logins, hasLength(1));
        expect(find.text('Finish sign-in in browser'), findsOneWidget);
        expectVisibleCard(tester);
        final button = tester.widget<FilledButton>(
          find.widgetWithText(FilledButton, 'Cancel'),
        );
        expect(
          button.style!.backgroundColor!.resolve({}),
          AppTheme.light().colorScheme.error,
        );
        await tester.runAsync(() async {
          await tester.tap(find.text('Cancel'));
          await pumpEventQueue();
        });
        await tester.pumpAndSettle();
        expect(find.text("It looks like you're signed out."), findsOneWidget);
        await tester.tap(find.text('Sign in'));
        await tester.pumpAndSettle();
        await tester.runAsync(() async {
          await signIn.browser.receive(
            Uri.parse(BrowserAuth.callback).replace(
              queryParameters: {
                'state': logins.last.queryParameters['state']!,
                'code': 'code',
              },
            ),
          );
          await pumpEventQueue();
        });
        expect(signIn.account.account?.subject, 'member');
        expect(
          await const FlutterSecureStorage().read(
            key: 'buzz_hosted_session_organization_example_v1',
          ),
          contains('member'),
        );
        await tester.pumpAndSettle();
        expect(find.byType(ReconnectCapsule), findsNothing);
      },
    );
    for (final failure in [
      AccountFailure.storage,
      AccountFailure.unavailable,
    ]) {
      testWidgets('$failure keeps sign-in recovery available at $scale', (
        tester,
      ) async {
        signIn.failure = failure;
        await mount(tester, scale: scale);
        expect(find.text('Sign in failed. Try again.'), findsOneWidget);
        expectVisibleCard(tester);
        await tester.tap(find.text('Sign in'));
        await tester.pumpAndSettle();
        expect(logins, hasLength(1));
      });
    }
  }
  testWidgets('denied access retains both actions at large text sizes', (
    tester,
  ) async {
    await tester.runAsync(() async {
      await signIn.signIn();
      await signIn.browser.receive(
        Uri.parse(BrowserAuth.callback).replace(
          queryParameters: {
            'state': logins.last.queryParameters['state']!,
            'code': 'code',
          },
        ),
      );
      await pumpEventQueue();
    });
    expect(signIn.account.account?.subject, 'member');
    logins.clear();
    auth.failedRelay = 'https://organization.example';
    auth.failure = AccountFailure.denied;
    await mount(tester, scale: 2);
    expect(tester.takeException(), isNull);
    expect(find.text('Denied access to this community.'), findsOneWidget);
    expect(
      tester.getTopLeft(find.text('Use a different account')).dy,
      greaterThan(tester.getBottomLeft(find.text('Retry')).dy),
    );
    await tester.tap(find.text('Retry'));
    expect(session.retries, 1);
    await tester.tap(find.text('Use a different account'));
    await tester.pumpAndSettle();
    expect(logins, hasLength(1));
    expect(signIn.account.account, isNull);
    expect(find.text('Cancel'), findsOneWidget);
  });
  testWidgets('transient toast yields back to persistent sign-in prompt', (
    tester,
  ) async {
    await mount(tester);
    await tester.tap(find.text('Toast'));
    await tester.pumpAndSettle();
    expect(find.text('Saved'), findsOneWidget);
    expect(find.text('Sign in'), findsNothing);
    await tester.pump(const Duration(seconds: 4));
    await tester.pumpAndSettle();
    expect(find.text('Sign in'), findsOneWidget);
  });
}

// Check the painted clip, not just absence of RenderFlex overflow: a stadium
// can silently cut glyphs off even when Flutter reports no layout exception.
void expectVisibleCard(WidgetTester tester) {
  expect(tester.takeException(), isNull);
  final card = find
      .descendant(
        of: find.byType(ReconnectCapsule),
        matching: find.byType(PhysicalShape),
      )
      .first;
  final bounds = tester.getRect(card);
  expect(bounds.left, greaterThanOrEqualTo(0));
  expect(bounds.top, greaterThanOrEqualTo(0));
  expect(bounds.right, lessThanOrEqualTo(320));
  expect(bounds.bottom, lessThanOrEqualTo(568));
  final shape = tester.widget<PhysicalShape>(card);
  final path = shape.clipper.getClip(bounds.size);
  for (final element
      in find.descendant(of: card, matching: find.byType(Text)).evaluate()) {
    final rect = tester
        .getRect(find.byWidget(element.widget))
        .shift(-bounds.topLeft)
        .deflate(0.1);
    for (final point in [
      rect.topLeft,
      rect.topRight,
      rect.bottomLeft,
      rect.bottomRight,
    ]) {
      expect(
        path.contains(point),
        isTrue,
        reason: 'Text must fit inside the painted card',
      );
    }
  }
}
