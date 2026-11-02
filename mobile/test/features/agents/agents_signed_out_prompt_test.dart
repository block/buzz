import 'package:buzz/features/profile/user_profile_sheet.dart';
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
  }
}

void main() {
  testWidgets(
    'signed out of Builderlab, Agents asks to sign in instead of claiming desktop',
    (tester) async {
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

      const prompt = 'Sign in to Builderlab to manage remote agents';
      expect(find.text(prompt), findsOneWidget);
      expect(find.byKey(ValueKey(existingDesktopKeys.public)), findsOneWidget);
      expect(find.text('Manage on desktop'), findsNothing);

      // The Management section cannot claim the agent runs on the desktop.
      await tester.tap(find.byKey(ValueKey(existingDesktopKeys.public)));
      await tester.pumpAndSettle();
      expect(find.byType(UserProfileSheet), findsOneWidget);
      expect(find.textContaining('Keep Buzz open'), findsNothing);
      expect(find.textContaining('Sign in to Builderlab to see'), findsOne);
      await tester.tapAt(const Offset(5, 5));
      await tester.pumpAndSettle();

      await tester.tap(find.widgetWithText(TextButton, 'Sign in'));
      await tester.pump();
      expect(flow.reconnects, 1);

      // Signing in restores the desktop labelling for unregistered agents.
      h.registry.signedIn = true;
      await tester.pumpAndSettle();
      expect(find.text(prompt), findsNothing);
      expect(find.text('Manage on desktop'), findsOneWidget);
    },
    skip: !HostedEnvironment.enabled,
  );
}
