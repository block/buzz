import '../../helpers/two_finger_double_tap.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/features/pairing/add_community_page.dart';
import 'package:flutter/cupertino.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:buzz/shared/community/community_grid_option.dart';
import 'dart:async';
import 'package:buzz/onboarding_app_entry.dart';

import 'package:buzz/hosted_app_gate.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:buzz/shared/widgets/onboarding_step_page.dart';
import 'package:buzz/shared/widgets/bee_loading_mark.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/hosted_communities_fixture.dart';
import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('Add uses landing input and leaves room above the keyboard', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(393, 852);
    addTearDown(tester.view.reset);
    for (final reduced in [false, true]) {
      var pairingStarted = false;
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            theme: AppTheme.light(),
            builder: (context, child) => MediaQuery(
              data: MediaQuery.of(context).copyWith(disableAnimations: reduced),
              child: child!,
            ),
            home: HookBuilder(
              builder: (context) {
                final entering = useState(false);
                return AddCommunityPage(
                  enteringCommunityLink: entering.value,
                  onBack: () => entering.value = false,
                  onCommunityLink: () => entering.value = true,
                  onPairing: () => pairingStarted = true,
                  onScan: () {},
                  onOpenCommunity: (_) {},
                  onSubmitPairing: (_) {},
                );
              },
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Use a pairing code'), findsNothing);
      final firstTap = await tester.createGesture();
      await firstTap.down(
        const Offset(196, 400),
        timeStamp: const Duration(milliseconds: 100),
      );
      await firstTap.up(timeStamp: const Duration(milliseconds: 120));
      await tester.pump(const Duration(milliseconds: 50));
      final secondTap = await tester.createGesture();
      await secondTap.down(
        const Offset(196, 400),
        timeStamp: const Duration(milliseconds: 200),
      );
      await secondTap.up(timeStamp: const Duration(milliseconds: 220));
      await tester.pumpAndSettle();
      expect(find.text('Use a pairing code'), findsNothing);
      await twoFingerDoubleTap(tester, const Offset(196, 400));
      expect(find.text('Use a pairing code'), findsOneWidget);
      await twoFingerDoubleTap(tester, const Offset(196, 400));
      expect(find.text('Use a pairing code'), findsNothing);
      await twoFingerDoubleTap(tester, const Offset(196, 400));
      expect(pairingStarted, isFalse);
      expect(find.text('Use a pairing code').hitTestable(), findsOneWidget);
      expect(
        tester.getTopLeft(find.text('Use a pairing code')).dy,
        greaterThan(
          tester.getBottomLeft(find.text('Enter a community link')).dy,
        ),
      );
      await tester.tap(find.text('Use a pairing code'));
      expect(pairingStarted, isTrue);
      final action = find.byKey(const Key('add-community-link-action'));
      final button = tester.element(action);
      await tester.tap(find.text('Enter a community link'));
      await tester.pump();
      tester.view.viewInsets = const FakeViewPadding(bottom: 300);
      await tester.pump(const Duration(milliseconds: 400));
      await tester.pumpAndSettle();
      expect(tester.element(action), same(button));
      final input = tester.widget<CupertinoTextField>(
        find.byKey(const Key('community-link-input')),
      );
      expect(input.decoration, isNull);
      expect(input.focusNode!.hasFocus, isTrue);
      expect(852 - 300 - tester.getBottomLeft(action).dy, closeTo(12, 0.1));
      await tester.enterText(
        find.byKey(const Key('community-link-input')),
        'example.com',
      );
      await tester.binding.handlePopRoute();
      tester.view.viewInsets = const FakeViewPadding();
      await tester.pumpAndSettle();
      expect(find.text('Add a community'), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
    }
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('Home stays hidden until community choice and admission', (
    tester,
  ) async {
    addTearDown(tester.platformDispatcher.clearAccessibilityFeaturesTestValue);
    // Secure-storage mocks must remain in one widget-test zone.
    for (final reduced in [false, true]) {
      tester.platformDispatcher.accessibilityFeaturesTestValue =
          FakeAccessibilityFeatures(disableAnimations: reduced);
      FlutterSecureStorage.setMockInitialValues({});
      PendingDeepLinkNotifier.debugUriStreamOverride =
          const Stream<Uri>.empty();
      final fixture = HostedCommunitiesFixture();
      await fixture.initialize(widget: true);
      fixture.rows = [fixture.row('one'), fixture.row('two')];

      await tester.pumpWidget(
        WidgetHelpers.testable(
          disableAnimations: reduced,
          overrides: [hostedFlowProvider.overrideWithValue(fixture.flow)],
          child: const HostedAppGate(
            authenticated: true,
            child: Scaffold(body: Text('Home content')),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await fixture.flow.signInAccount();
      await tester.pumpAndSettle();
      expect(fixture.flow.stage, HostedFlowStage.chooseCommunities);
      expect(fixture.admitted, isEmpty);
      expect(find.text('Home content'), findsNothing);
      expect(find.text('Select a community'), findsOneWidget);
      expect(find.byType(BottomSheet), findsNothing);
      expect(find.byTooltip('Back'), findsOneWidget);
      // Cancelling restore must not open Add after the pending request returns.
      final cancelledRestore = Completer<void>();
      fixture.beforeAdmission = (_) => cancelledRestore.future;
      await tester.ensureVisible(
        find.byKey(const Key('community-switcher-add')),
      );
      await tester.tap(find.byKey(const Key('community-switcher-add')));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.byTooltip('Back'), findsNothing);
      expect(find.byType(BeeLoadingMark), findsOneWidget);
      await tester.binding.handlePopRoute();
      await tester.pump();
      expect(fixture.flow.stage, HostedFlowStage.working);
      // External cancellation still invalidates a pending Add restoration.
      fixture.flow.backToCommunityChoices();
      cancelledRestore.complete();
      await tester.pumpAndSettle();
      expect(find.byType(OnboardingAppEntry), findsNothing);
      expect(find.text('Select a community'), findsOneWidget);
      fixture.beforeAdmission = null;
      // Add restores existing memberships, stays opaque, and returns to the
      // picker on Back without the disposed Add route cancelling discovery.
      await tester.ensureVisible(
        find.byKey(const Key('community-switcher-add')),
      );
      await tester.tap(find.byKey(const Key('community-switcher-add')));
      await tester.pumpAndSettle();
      expect(fixture.admitted, hasLength(2));
      expect(find.byType(OnboardingAppEntry), findsOneWidget);
      expect(find.text('Home content').hitTestable(), findsNothing);
      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(find.byType(OnboardingAppEntry), findsNothing);
      expect(find.text('Select a community'), findsOneWidget);
      expect(fixture.flow.stage, HostedFlowStage.chooseCommunities);
      fixture.admitted.clear();

      final admission = Completer<void>();
      fixture.beforeAdmission = (_) => admission.future;
      await tester.tap(find.byType(CommunityGridOption).last);
      await tester.pump();
      // Back must be blocked even while the outgoing picker is mounted.
      await tester.binding.handlePopRoute();
      await tester.pump();
      expect(fixture.flow.stage, HostedFlowStage.working);
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.text('Home content'), findsNothing);
      expect(find.text('Joining your community'), findsOneWidget);
      expect(find.byType(CircularProgressIndicator), findsNothing);
      expect(find.byTooltip('Back'), findsNothing);
      expect(
        tester.getBottomLeft(find.byType(BeeLoadingMark)).dy,
        lessThan(tester.getTopLeft(find.text('Joining your community')).dy),
      );
      final bee = tester.widget<CustomPaint>(
        find.descendant(
          of: find.byType(BeeLoadingMark),
          matching: find.byType(CustomPaint),
        ),
      );
      expect(bee.size.width, 80);
      final mark = tester.getRect(find.byType(BeeLoadingMark));
      expect(
        mark.center.dx,
        tester.view.physicalSize.width / tester.view.devicePixelRatio / 2,
      );
      expect(
        (mark.top +
                tester.getRect(find.text('Joining your community')).bottom) /
            2,
        tester.view.physicalSize.height / tester.view.devicePixelRatio / 2,
      );
      expect(
        tester.getTopLeft(find.text('Joining your community')).dy - mark.bottom,
        24,
      );
      expect(
        tester.widget<Text>(find.text('Joining your community')).textAlign,
        TextAlign.center,
      );
      expect(
        MediaQuery.disableAnimationsOf(
          tester.element(find.byType(BeeLoadingMark)),
        ),
        reduced,
      );
      if (reduced) {
        await tester.pumpAndSettle(
          const Duration(milliseconds: 100),
          EnginePhase.sendSemanticsUpdate,
          const Duration(seconds: 1),
        );
      }
      expect(fixture.admitted, isEmpty);
      final opening = Completer<void>();
      fixture.beforeOpen = () => opening.future;
      admission.complete();
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.text('Home content'), findsNothing);
      opening.complete();
      await tester.pumpAndSettle();
      expect(fixture.admitted, [
        'two.communities.buzz.xyz',
        'one.communities.buzz.xyz',
      ]);
      expect(fixture.openedCommunity, 'wss://two.communities.buzz.xyz');
      expect(find.text('Home content').hitTestable(), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
      await fixture.dispose();
    }
    PendingDeepLinkNotifier.debugUriStreamOverride = null;
  }, skip: !HostedEnvironment.enabled);

  testWidgets('step actions remain reachable with large text and keyboard', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(320, 640);
    addTearDown(tester.view.reset);
    var continued = false;
    await tester.pumpWidget(
      MaterialApp(
        home: MediaQuery(
          data: const MediaQueryData(
            size: Size(320, 640),
            textScaler: TextScaler.linear(2),
            viewInsets: EdgeInsets.only(bottom: 260),
          ),
          child: OnboardingStepPage(
            title: 'Bring your identity with you',
            subtitle: 'Use the identity from your other device to continue.',
            onBack: () {},
            content: const Text('Your saved identity stays on your device.'),
            actions: [
              FilledButton(
                onPressed: () => continued = true,
                child: const Text('Continue'),
              ),
            ],
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.text('Continue'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Continue'));
    expect(continued, isTrue);
    expect(tester.takeException(), isNull);
  });
}
