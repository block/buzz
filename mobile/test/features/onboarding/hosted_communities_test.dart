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
        await tester.tap(find.text('Other options'));
        await tester.pumpAndSettle();
        tester.view.physicalSize = const Size(420, 900);
        await tester.pumpAndSettle();
        final linkCard = find.ancestor(
          of: find.text('Enter a community link').hitTestable(),
          matching: find.byType(InkWell),
        );
        expect(tester.getBottomRight(linkCard).dy, closeTo(876, 1));
        final scanCard = find.ancestor(
          of: find.text('Scan a QR code').hitTestable(),
          matching: find.byType(InkWell),
        );
        final subtitle = find
            .text('Scan a QR code or enter a community link to continue.')
            .hitTestable();
        expect(
          tester.getTopLeft(scanCard).dy - tester.getBottomLeft(subtitle).dy,
          closeTo(24, 1),
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
        expect(find.text('Scan a QR code').hitTestable(), findsOneWidget);
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
        final developerTrigger = find
            .byKey(const Key('developer-pairing-trigger'))
            .hitTestable();
        await tester.tap(developerTrigger);
        await tester.pump(const Duration(milliseconds: 50));
        await tester.tap(developerTrigger);
        await tester.pumpAndSettle();
        expect(
          find
              .text('Paste the pairing code from your desktop app.')
              .hitTestable(),
          findsOneWidget,
        );
        await tester.tap(find.text('Back').hitTestable());
        await tester.pumpAndSettle();
        expect(find.text('Scan a QR code').hitTestable(), findsOneWidget);
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
        expect(
          find.text('Sign in with builderlab').hitTestable(),
          findsOneWidget,
        );
        expect(find.text('Enter a community link').hitTestable(), findsNothing);
        expect(find.text('Scan a QR code'), findsNothing);
        expect(find.text('Use pairing code'), findsNothing);
        await tester.tap(find.text('Other options'));
        await tester.pumpAndSettle();
        expect(
          find.text('Enter a community link').hitTestable(),
          findsOneWidget,
        );
        expect(find.text('Scan a QR code').hitTestable(), findsOneWidget);
        expect(find.text('Use pairing code').hitTestable(), findsNothing);
        await tester.tap(find.byType(BackButton));
        await tester.pumpAndSettle();
        expect(find.text('Enter a community link').hitTestable(), findsNothing);
        await tester.tap(find.text('Other options'));
        await tester.pumpAndSettle();
        await tester.binding.handlePopRoute();
        await tester.pumpAndSettle();
        expect(find.text('Other options').hitTestable(), findsOneWidget);
        expect(find.text('Enter a community link').hitTestable(), findsNothing);
        await tester.tap(find.text('Sign in with builderlab'));
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 300));
        expect(f.browser.startedIntent, {'type': 'account'});
        expect(
          find.bySemanticsLabel('Cancel sign-in').hitTestable(),
          findsOneWidget,
        );
        await tester.tap(find.bySemanticsLabel('Cancel sign-in').hitTestable());
        await tester.pumpAndSettle();
        expect(
          find.text('Sign in with builderlab').hitTestable(),
          findsOneWidget,
        );
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
