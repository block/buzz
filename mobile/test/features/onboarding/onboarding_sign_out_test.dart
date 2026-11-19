import 'package:buzz/onboarding_app_entry.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/hosted_communities_fixture.dart';
import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('signed-in landing confirms sign-out and keeps saved identity', (
    tester,
  ) async {
    if (!HostedEnvironment.enabled) return;
    await tester.binding.setSurfaceSize(const Size(440, 956));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    FlutterSecureStorage.setMockInitialValues({});
    final fixture = HostedCommunitiesFixture();
    await fixture.initialize();
    addTearDown(fixture.dispose);
    await fixture.flow.leave(noCommunities: true);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        disableAnimations: true,
        overrides: [hostedFlowProvider.overrideWithValue(fixture.flow)],
        child: const OnboardingAppEntry(),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Signed in as owner@example.com'), findsOneWidget);

    // Back within a form navigates without offering sign-out.
    await tester.tap(find.text('Enter community link'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Back'));
    await tester.pumpAndSettle();
    expect(find.text('Sign out of BuilderLab?'), findsNothing);
    expect(fixture.account.account, isNotNull);

    await tester.tap(find.byTooltip('Back'));
    await tester.pumpAndSettle();
    expect(find.text('Sign out of BuilderLab?'), findsOneWidget);
    expect(find.textContaining('owner@example.com'), findsNWidgets(2));
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(fixture.account.account, isNotNull);
    expect(find.text('Add a community'), findsOneWidget);

    // System Back follows the same confirmation path.
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    await tester.tap(find.text('Sign out'));
    await tester.pumpAndSettle();
    expect(find.text('Add a community'), findsNothing);
    expect(find.text('or sign in with email').hitTestable(), findsOneWidget);
    expect(fixture.account.account, isNull);
    expect(await fixture.account.restore(), isNull);
    expect((await fixture.store.read())!.pubkey, fixture.keys.public);
    expect((await fixture.store.read())!.backup, isNotNull);

    await tester.tap(find.text('or sign in with email').hitTestable());
    await tester.pumpAndSettle();
    expect(fixture.browser.startedIntent, {'type': 'account'});
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pumpAndSettle();
  });
}
