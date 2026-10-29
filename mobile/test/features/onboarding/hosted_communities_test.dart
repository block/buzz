import 'package:buzz/shared/community/community_grid_option.dart';
import 'dart:ui' as ui;

import 'package:buzz/shared/hosted/hosted_community_creation.dart';
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
import 'package:flutter/services.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/hosted_communities_fixture.dart';
import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets(
    'cancel sign-in and resolve batch conflicts without replacing saved identities',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      // Keep secure-storage operations in the same test zone.
      for (final reduced in [false, true]) {
        tester.view.devicePixelRatio = 1;
        tester.view.physicalSize = const Size(320, 640);

        FlutterSecureStorage.setMockInitialValues({});
        PendingDeepLinkNotifier.debugUriStreamOverride =
            const Stream<Uri>.empty();
        final f = HostedCommunitiesFixture();
        await f.initialize(widget: true);
        f.expired = true;

        await tester.pumpWidget(
          WidgetHelpers.testable(
            disableAnimations: reduced,
            child: const HostedAppGate(
              authenticated: false,
              child: OnboardingAppEntry(),
            ),
            overrides: [hostedFlowProvider.overrideWithValue(f.flow)],
          ),
        );
        await tester.pumpAndSettle();
        await revealOptionsWithSemantics(tester);
        await tester.pumpAndSettle();
        tester.view.physicalSize = const Size(420, 900);
        await tester.pumpAndSettle();
        final linkCard = find.ancestor(
          of: find.text('Enter a community link').hitTestable(),
          matching: find.byType(InkWell),
        );
        expect(tester.getBottomRight(linkCard).dy, closeTo(876, 1));
        final scanCard = find.ancestor(
          of: find.text('Use a pairing code').hitTestable(),
          matching: find.byType(InkWell),
        );
        final subtitle = find
            .text('Enter a community link or use a pairing code to continue.')
            .hitTestable();
        expect(
          tester.getTopLeft(scanCard).dy - tester.getBottomLeft(subtitle).dy,
          // The transitioning heading reserves the taller of both text blocks.
          greaterThanOrEqualTo(24),
        );
        tester.view.physicalSize = const Size(320, 640);
        await tester.pumpAndSettle();
        final optionTop = tester
            .getTopLeft(find.text('Enter a community link').hitTestable())
            .dy;
        final scanTop = tester.getTopLeft(scanCard).dy;
        await tester.tap(find.text('Enter a community link').hitTestable());
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 30));
        if (!reduced) {
          // Keyboard setup must finish before any heading/surface movement.
          expect(
            tester
                .getTopLeft(find.byKey(const Key('connection-options-motion')))
                .dy,
            scanTop,
          );
          await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
            'buzz/community_keyboard',
            const StandardMethodCodec().encodeMethodCall(
              const MethodCall('willShow'),
            ),
            (_) {},
          );
          await tester.pump();
          await tester.pump(const Duration(milliseconds: 30));
          final scanLayer = find.byKey(const Key('connection-options-motion'));
          expect(tester.getTopLeft(scanLayer).dy, greaterThan(scanTop));
          final formOpacity = tester.widget<Opacity>(
            find
                .descendant(
                  of: find.byKey(const Key('community-form-motion')),
                  matching: find.byType(Opacity),
                )
                .first,
          );
          expect(formOpacity.opacity, greaterThan(0));
          final surface = find.byWidgetPredicate(
            (widget) => widget is LayoutId && widget.id == 'surface',
          );
          final action = find.byWidgetPredicate(
            (widget) => widget is LayoutId && widget.id == 'action',
          );
          expect(tester.getRect(surface), tester.getRect(action));
        }
        await tester.pump(const Duration(milliseconds: 50));
        await tester.binding.handlePopRoute();
        await tester.pumpAndSettle();
        expect(tester.testTextInput.isVisible, isFalse);
        expect(find.text('Use a pairing code').hitTestable(), findsOneWidget);
        await tester.tap(find.text('Enter a community link').hitTestable());
        await tester.pump();
        // A hardware keyboard produces no software-keyboard notification.
        await tester.pump(const Duration(milliseconds: 350));
        await tester.pumpAndSettle();
        expect(
          tester
              .getTopLeft(find.text('Enter a community link').hitTestable())
              .dy,
          lessThan(optionTop - 100),
        );
        expect(tester.testTextInput.isVisible, isTrue);
        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        await tester.pumpAndSettle();
        final field = find.byKey(const Key('community-link-input'));
        await tester.ensureVisible(field);
        expect(field.hitTestable(), findsOneWidget);
        expect(
          tester
              .widget<EditableText>(
                find.descendant(of: field, matching: find.byType(EditableText)),
              )
              .focusNode
              .hasFocus,
          isTrue,
        );
        expect(tester.takeException(), isNull);
        tester.view.viewInsets = const FakeViewPadding();
        await tester.binding.handlePopRoute();
        await tester.pumpAndSettle();
        expect(tester.testTextInput.isVisible, isFalse);
        expect(find.text('Use pairing code'), findsNothing);
        await tester.tap(find.text('Use a pairing code').hitTestable());
        await tester.pumpAndSettle();
        expect(
          find
              .text('Paste the pairing code from your desktop app.')
              .hitTestable(),
          findsOneWidget,
        );
        await tester.tap(find.text('Back').hitTestable());
        await tester.pumpAndSettle();
        expect(
          find.text('or sign in with email').hitTestable(),
          findsOneWidget,
        );
        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pumpAndSettle();
        await f.dispose();
        tester.view.reset();
      }
      debugDefaultTargetPlatformOverride = null;
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
        f.expired = true;
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
        expect(find.text('Scan a QR code').hitTestable(), findsOneWidget);
        expect(find.text('Enter a community link').hitTestable(), findsNothing);
        expect(find.text('Sign in with email').hitTestable(), findsNothing);
        expect(find.text('Use pairing code'), findsNothing);
        await revealOptions(tester);
        await tester.pumpAndSettle();
        expect(
          find.text('Enter a community link').hitTestable(),
          findsOneWidget,
        );
        expect(find.text('Use a pairing code').hitTestable(), findsOneWidget);
        expect(find.text('Use pairing code').hitTestable(), findsNothing);
        await tester.tap(find.byType(BackButton));
        await tester.pumpAndSettle();
        expect(find.text('Enter a community link').hitTestable(), findsNothing);
        await revealOptions(tester);
        await tester.pumpAndSettle();
        await tester.binding.handlePopRoute();
        await tester.pumpAndSettle();
        expect(
          find.text('or sign in with email').hitTestable(),
          findsOneWidget,
        );
        expect(find.text('Enter a community link').hitTestable(), findsNothing);
        await tester.tap(find.text('or sign in with email').hitTestable());
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 300));
        expect(f.browser.startedIntent, {'type': 'account'});
        expect(
          find.bySemanticsLabel('Cancel sign-in').hitTestable(),
          findsOneWidget,
        );
        await tester.tap(find.bySemanticsLabel('Cancel sign-in').hitTestable());
        await tester.pumpAndSettle();
        expect(find.text('Scan a QR code').hitTestable(), findsOneWidget);
        // Target sign-in remains pending when an Android Custom Tab closes.
        await f.flow.start(HostedCreateCommunity('new-community'));
        await tester.pumpAndSettle();
        expect(f.flow.accountOnly, isFalse);
        expect(f.flow.stage, HostedFlowStage.signIn);
        expect(find.text('Cancel sign-in').hitTestable(), findsOneWidget);
        expect(find.text('Scan a QR code').hitTestable(), findsNothing);
        await tester.tap(find.text('Cancel sign-in').hitTestable());
        await tester.pumpAndSettle();
        expect(f.flow.stage, HostedFlowStage.idle);
        expect(find.text('Scan a QR code').hitTestable(), findsOneWidget);
        f.expired = false;
        await f.flow.signInAccount(openBrowser: false);
        await tester.pumpAndSettle();
        f.expired = true;
        await tester.tap(find.text('Continue with Builderlab'));
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 300));
        expect(f.flow.stage, HostedFlowStage.signIn);
        expect(f.browser.startedIntent, {'type': 'account'});
        expect(find.byType(HostedConnectPage), findsNothing);
        expect(
          find.bySemanticsLabel('Cancel sign-in').hitTestable(),
          findsOneWidget,
        );
        expect(find.text('Enter a community link').hitTestable(), findsNothing);
        f.expired = false;
        await tester.tap(find.bySemanticsLabel('Cancel sign-in').hitTestable());
        await tester.pumpAndSettle();
        expect(f.flow.stage, HostedFlowStage.idle);
        await tester.pumpAndSettle();
        expect(find.text('Continue with Builderlab'), findsOneWidget);
        expect(find.byType(HostedConnectPage), findsNothing);
        await tester.tap(find.text('Continue with Builderlab'));
        await tester.pumpAndSettle();
        if (f.flow.stage == HostedFlowStage.chooseCommunities) {
          expect(find.byType(CommunityGridOption), findsWidgets);
          await tester.tap(find.byType(CommunityGridOption).first);
          await tester.pumpAndSettle();
        }
        expect(find.byType(CommunityGridOption), findsNothing);
        expect(f.admitted, [
          'one.communities.buzz.xyz',
          'two.communities.buzz.xyz',
        ]);
        expect(find.byType(HostedConnectPage), findsNothing);
        f.admitted.clear();
        f.beforeAdmission = (name) async {
          if (name == 'two.communities.buzz.xyz') {
            throw const SavedCommunityIdentityConflict();
          }
        };
        await tester.tap(find.text('Continue with Builderlab'));
        await tester.pumpAndSettle();
        if (f.flow.stage == HostedFlowStage.chooseCommunities) {
          expect(find.byType(CommunityGridOption), findsWidgets);
          await tester.tap(find.byType(CommunityGridOption).first);
          await tester.pumpAndSettle();
        }
        expect(find.text('Keep saved identity and skip'), findsOneWidget);
        expect(find.text('1 community added and kept.'), findsOneWidget);
        await tester.tap(find.text('Keep saved identity and skip'));
        await tester.pumpAndSettle();
        expect(f.admitted, ['one.communities.buzz.xyz']);
        expect(find.byType(HostedConnectPage), findsNothing);
        f.admitted.clear();
        f.beforeAdmission = (name) async {
          if (name == 'two.communities.buzz.xyz') throw AccountFailure.rejected;
        };
        await tester.tap(find.text('Continue with Builderlab'));
        await tester.pumpAndSettle();
        if (f.flow.stage == HostedFlowStage.chooseCommunities) {
          expect(find.byType(CommunityGridOption), findsWidgets);
          await tester.tap(find.byType(CommunityGridOption).first);
          await tester.pumpAndSettle();
        }
        expect(find.text('1 community added and kept.'), findsOneWidget);
        expect(find.text('Try again').hitTestable(), findsNothing);
        await tester.tap(find.text('Back to choices'));
        await tester.pumpAndSettle();
        expect(find.byType(CommunityGridOption), findsNWidgets(2));
        expect(
          find.descendant(
            of: find.byType(CommunityGridOption),
            matching: find.text('two.communities.buzz.xyz'),
          ),
          findsWidgets,
        );
      }
    },
    skip: HostedEnvironment.authDomains.isEmpty,
  );
}

Future<void> revealOptions(WidgetTester tester) async {
  // Ordinary double taps must not expose the hidden connection options.
  await tester.tapAt(const Offset(24, 120));
  await tester.pump(const Duration(milliseconds: 50));
  await tester.tapAt(const Offset(24, 120));
  await tester.pumpAndSettle();
  expect(find.text('Use a pairing code').hitTestable(), findsNothing);

  for (var tap = 0; tap < 2; tap++) {
    final first = await tester.createGesture(pointer: 11);
    final second = await tester.createGesture(pointer: 12);
    await first.down(
      const Offset(24, 120),
      timeStamp: Duration(milliseconds: tap * 70),
    );
    await second.down(
      const Offset(70, 120),
      timeStamp: Duration(milliseconds: tap * 70),
    );
    await tester.pump(const Duration(milliseconds: 20));
    await first.up(timeStamp: Duration(milliseconds: tap * 70 + 20));
    await second.up(timeStamp: Duration(milliseconds: tap * 70 + 20));
    await tester.pump(const Duration(milliseconds: 50));
  }
}

Future<void> revealOptionsWithSemantics(WidgetTester tester) async {
  final handle = tester.ensureSemantics();
  try {
    await tester.pump();
    final wordmark = tester.getSemantics(find.bySemanticsLabel('Buzz'));
    final data = wordmark.getSemanticsData();
    final actionIds = data.customSemanticsActionIds!;
    expect(actionIds, hasLength(1));
    expect(
      CustomSemanticsAction.getAction(actionIds.single)!.label,
      'More connection options',
    );
    // The action belongs to the wordmark, never an unlabelled page-sized stop.
    expect(data.label, 'Buzz');
    expect(wordmark.rect.height, lessThan(200));
    wordmark.owner!.performAction(
      wordmark.id,
      ui.SemanticsAction.customAction,
      actionIds.single,
    );
    await tester.pumpAndSettle();
    expect(find.text('Enter a community link').hitTestable(), findsOneWidget);
    expect(find.text('Use a pairing code').hitTestable(), findsOneWidget);
  } finally {
    handle.dispose();
  }
}
