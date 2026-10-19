import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/features/onboarding/hosted_connect_page.dart';
import 'package:buzz/hosted_app_gate.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:buzz/onboarding_app_entry.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_community_connection.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/hosted_communities_fixture.dart';
import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets(
    'cancel sign-in and resolve batch conflicts without replacing saved identities',
    (tester) async {
      await tester.binding.setSurfaceSize(const Size(440, 956));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      // Keep the process-wide secure-store queue in one widget-test zone.
      {
        FlutterSecureStorage.setMockInitialValues({});
        PendingDeepLinkNotifier.debugUriStreamOverride =
            const Stream<Uri>.empty();
        final f = HostedCommunitiesFixture();
        await f.initialize(widget: true);
        f.rows = [f.row('one'), f.row('two')];
        addTearDown(() async {
          PendingDeepLinkNotifier.debugUriStreamOverride = null;
          await f.dispose();
        });
        await tester.pumpWidget(
          WidgetHelpers.testable(
            disableAnimations: true,
            child: const HostedAppGate(
              authenticated: false,
              child: OnboardingAppEntry(),
            ),
            overrides: [hostedFlowProvider.overrideWithValue(f.flow)],
          ),
        );
        await tester.pumpAndSettle();
        f.expired = true;
        await tester.tap(find.text('Continue with Builderlab'));
        await tester.pump();
        await tester.pumpAndSettle();
        expect(f.flow.stage, HostedFlowStage.signIn);
        expect(f.browser.startedIntent, {'type': 'account'});
        expect(find.byType(HostedConnectPage), findsNothing);
        expect(find.text('Cancel sign-in').hitTestable(), findsOneWidget);
        expect(find.text('Enter a community link'), findsOneWidget);
        f.expired = false;
        await tester.tap(find.text('Cancel sign-in').hitTestable());
        await tester.pumpAndSettle();
        expect(f.flow.stage, HostedFlowStage.idle);
        await tester.pumpAndSettle();
        expect(find.text('Continue with Builderlab'), findsOneWidget);
        expect(find.byType(HostedConnectPage), findsNothing);
        await tester.tap(find.text('Continue with Builderlab'));
        await tester.pumpAndSettle();
        expect(find.byType(CheckboxListTile), findsNothing);
        expect(f.admitted, ['one', 'two']);
        expect(find.byType(HostedConnectPage), findsNothing);
        f.admitted.clear();
        f.beforeAdmission = (name) async {
          if (name == 'two') throw const SavedCommunityIdentityConflict();
        };
        await tester.tap(find.text('Continue with Builderlab'));
        await tester.pumpAndSettle();
        expect(find.text('Keep saved identity and skip'), findsOneWidget);
        expect(find.text('1 community added and kept.'), findsOneWidget);
        await tester.tap(find.text('Keep saved identity and skip'));
        await tester.pumpAndSettle();
        expect(f.admitted, ['one']);
        expect(find.byType(HostedConnectPage), findsNothing);
        f.admitted.clear();
        f.beforeAdmission = (name) async {
          if (name == 'two') throw AccountFailure.rejected;
        };
        await tester.tap(find.text('Continue with Builderlab'));
        await tester.pumpAndSettle();
        expect(find.text('1 community added and kept.'), findsOneWidget);
        expect(find.text('Try again').hitTestable(), findsNothing);
        await tester.tap(find.text('Back to choices'));
        await tester.pumpAndSettle();
        expect(find.byType(CheckboxListTile), findsOneWidget);
        expect(find.text('two'), findsOneWidget);
      }
    },
    skip: HostedEnvironment.authDomains.isEmpty,
  );
}
