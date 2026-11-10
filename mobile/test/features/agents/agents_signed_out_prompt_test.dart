import 'package:buzz/features/profile/user_profile_sheet.dart';
import 'package:buzz/shared/widgets/buzz_toast.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:buzz/shared/hosted/hosted_identity_service.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'agents_harness.dart';

class _Flow extends HostedFlow {
  _Flow(AgentsHarness h)
    : super(
        account: h.registry,
        store: AccountIdentityStore(),
        browser: BrowserAuth(),
        identity: HostedIdentityService(
          account: h.registry,
          store: AccountIdentityStore(),
        ),
        enterCommunity: (_, _, _, _) async {},
      );
  int reconnects = 0;

  @override
  Future<void> reconnectAccount({bool changeAccount = false}) async {
    reconnects++;
    reconnecting = true;
    stage = HostedFlowStage.signIn;
    notifyListeners();
  }

  @override
  Future<void> leave({bool noCommunities = false}) async {
    reconnecting = false;
    stage = HostedFlowStage.idle;
    notifyListeners();
  }
}

void main() {
  testWidgets(
    'signed out of Builderlab, Agents asks to sign in instead of claiming desktop',
    (tester) async {
      tester.view.physicalSize = const Size(390, 844);
      tester.view.devicePixelRatio = 1;
      tester.view.padding = FakeViewPadding(top: 59, bottom: 34);
      tester.view.viewPadding = FakeViewPadding(top: 59, bottom: 34);
      addTearDown(tester.view.reset);
      final h = await AgentsHarness.create();
      final flow = _Flow(h);
      addTearDown(() async {
        flow.dispose();
        await flow.browser.dispose();
      });
      h.registry.signedIn = false;
      await h.open(
        tester,
        directory: [existingDesktopKeys.public],
        overrides: [hostedFlowProvider.overrideWithValue(flow)],
      );

      const prompt = 'Reconnect to Builderlab';
      expect(find.text(prompt), findsOneWidget);
      expect(find.byType(ReconnectCapsule), findsOneWidget);
      expect(find.text('Sign in').hitTestable(), findsOneWidget);

      // The prompt belongs to this visit, not the global toast overlay.
      await tester.pump(const Duration(seconds: 10));
      expect(find.text(prompt), findsOneWidget);
      await tester.tap(find.byTooltip('Close agents'));
      await tester.pumpAndSettle();
      expect(find.text(prompt), findsNothing);
      await tester.tap(find.text('Open Agents'));
      await tester.pumpAndSettle();
      expect(find.text(prompt), findsOneWidget);
      expect(find.byKey(ValueKey(existingDesktopKeys.public)), findsOneWidget);
      expect(find.text('Manage on desktop'), findsNothing);

      // The capsule scrolls with the cards rather than pinning over them.
      tester.view.physicalSize = const Size(390, 400);
      await tester.pumpAndSettle();
      final promptBefore = tester.getTopLeft(find.text(prompt)).dy;
      final card = find.byKey(ValueKey(existingDesktopKeys.public));
      final cardBefore = tester.getTopLeft(card).dy;
      await tester.drag(
        find.byType(CustomScrollView).first,
        const Offset(0, -120),
      );
      await tester.pumpAndSettle();
      final promptDelta =
          tester.getTopLeft(find.text(prompt)).dy - promptBefore;
      expect(promptDelta, lessThan(-20));
      expect(tester.getTopLeft(card).dy - cardBefore, closeTo(promptDelta, 1));
      tester.view.physicalSize = const Size(390, 844);
      await tester.pumpAndSettle();

      // The Management section cannot claim the agent runs on the desktop.
      await tester.tap(find.byKey(ValueKey(existingDesktopKeys.public)));
      await tester.pumpAndSettle();
      expect(find.byType(UserProfileSheet), findsOneWidget);
      expect(find.textContaining('Keep Buzz open'), findsNothing);
      expect(find.textContaining('Sign in to Builderlab to see'), findsOne);
      await tester.tapAt(const Offset(5, 5));
      await tester.pumpAndSettle();

      await tester.tap(find.widgetWithText(FilledButton, 'Sign in'));
      await tester.pump();
      expect(flow.reconnects, 1);
      expect(find.text('Finish sign-in in browser'), findsOneWidget);
      await tester.tap(find.widgetWithText(FilledButton, 'Cancel'));
      await tester.pumpAndSettle();
      expect(find.text(prompt), findsOneWidget);
      await tester.tap(find.widgetWithText(FilledButton, 'Sign in'));
      await tester.pumpAndSettle();
      expect(flow.reconnects, 2);

      // Signing in restores the desktop labelling for unregistered agents.
      h.registry.signedIn = true;
      await tester.pumpAndSettle();
      expect(find.text(prompt), findsNothing);
      expect(find.text('Manage on desktop'), findsOneWidget);
    },
    skip: !HostedEnvironment.enabled,
  );
}
