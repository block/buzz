import 'package:buzz/features/channels/deep_link_dispatcher.dart';
import 'package:buzz/hosted_app_gate.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/hosted/hosted_identity_service.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _Flow extends HostedFlow {
  _Flow(BuilderlabAccountClient account, AccountIdentityStore store)
    : super(
        account: account,
        store: store,
        browser: BrowserAuth(),
        identity: HostedIdentityService(account: account, store: store),
        enterCommunity: (_, _, _, _) async {},
      );
  BuzzDeepLink? started;
  @override
  Future<void> initialize() async {}
  @override
  Future<void> start(BuzzDeepLink target, {bool resumePairing = false}) async {
    started = target;
  }
}

void main() {
  testWidgets('app dispatch keeps staging invites out of legacy generation', (
    tester,
  ) async {
    FlutterSecureStorage.setMockInitialValues({});
    PendingDeepLinkNotifier.debugUriStreamOverride = const Stream<Uri>.empty();
    final account = BuilderlabAccountClient();
    final flow = _Flow(account, AccountIdentityStore());
    addTearDown(() async {
      PendingDeepLinkNotifier.debugUriStreamOverride = null;
      flow.dispose();
      await flow.browser.dispose();
      account.dispose();
    });
    await tester.pumpWidget(
      ProviderScope(
        overrides: [hostedFlowProvider.overrideWithValue(flow)],
        child: const MaterialApp(
          home: HostedAppGate(
            authenticated: false,
            child: Scaffold(body: Text('entry')),
          ),
        ),
      ),
    );
    await tester.pump();
    final dispatcher = tester.widget<DeepLinkDispatcher>(
      find.byType(DeepLinkDispatcher),
    );
    final context = tester.element(find.byType(DeepLinkDispatcher));
    const staging = InviteDeepLink(
      relayUrl: 'wss://team.communities.public.builderlabstaging.xyz',
      code: 'fixture',
    );
    final pending = dispatcher.onboardingHandler!(context, staging);
    await tester.pumpAndSettle();
    if (HostedEnvironment.origin.host == 'app.builderlabstaging.xyz') {
      expect(await pending, isTrue);
      expect(flow.started, staging);
    } else {
      expect(find.text('Community setup unavailable'), findsOneWidget);
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();
      expect(await pending, isTrue);
      expect(flow.started, isNull);
    }
    final dotted = dispatcher.onboardingHandler!(
      context,
      const InviteDeepLink(
        relayUrl: 'wss://team.communities.public.builderlabstaging.xyz.',
        code: 'fixture',
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Community setup unavailable'), findsOneWidget);
    await tester.tap(find.text('Close'));
    await tester.pumpAndSettle();
    expect(await dotted, isTrue);
    expect(
      await dispatcher.onboardingHandler!(
        context,
        const InviteDeepLink(
          relayUrl: 'wss://community.example.org',
          code: 'fixture',
        ),
      ),
      isFalse,
    );
  });
}
