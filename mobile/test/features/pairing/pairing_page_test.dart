import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:local_auth/local_auth.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:buzz/features/pairing/pairing_page.dart';
import 'package:buzz/features/pairing/pairing_page/onboarding_wordmark.dart';
import 'package:buzz/features/pairing/pairing_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/security/sensitive_action_authorizer.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/buzz_loading_indicator.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  group('PairingPage', () {
    testWidgets('renders branding and progressive pairing actions', (
      tester,
    ) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(child: const PairingPage()),
      );

      expect(find.bySemanticsLabel('Buzz'), findsOneWidget);
      expect(find.text('Welcome to Buzz'), findsNothing);
      expect(
        find.text(
          'Your people, your agents, your projects —\nall in one place.',
        ),
        findsOneWidget,
      );
      expect(find.text('Scan a QR code'), findsOneWidget);
      expect(find.text('Use pairing code'), findsOneWidget);
      expect(find.text('Connect'), findsNothing);
      expect(find.byType(TextField), findsNothing);
    });

    testWidgets('uses compact desktop-style onboarding actions', (
      tester,
    ) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(child: const PairingPage()),
      );

      final scanButton = tester.getSize(
        find.widgetWithText(FilledButton, 'Scan a QR code'),
      );
      final pairingCodeButton = tester.getSize(
        find.widgetWithText(TextButton, 'Use pairing code'),
      );

      final glassFinder = find.widgetWithText(FilledButton, 'Scan a QR code');
      expect(
        find.descendant(of: glassFinder, matching: find.byType(BackdropFilter)),
        findsOneWidget,
      );
      expect(scanButton.height, greaterThanOrEqualTo(44));
      expect(pairingCodeButton.height, greaterThanOrEqualTo(44));
      expect(
        tester
            .widget<TextButton>(
              find.widgetWithText(TextButton, 'Use pairing code'),
            )
            .style!
            .backgroundColor!
            .resolve({}),
        Colors.transparent,
      );
      expect(scanButton.width, lessThan(440));
      expect(pairingCodeButton.width, lessThan(440));
      expect(find.byType(OutlinedButton), findsNothing);
    });

    testWidgets(
      'docks welcome actions and keeps pairing reachable with keyboard',
      (tester) async {
        tester.view.devicePixelRatio = 1;
        tester.view.physicalSize = const Size(390, 844);
        tester.view.padding = const FakeViewPadding(top: 59, bottom: 34);
        addTearDown(tester.view.reset);
        await tester.pumpWidget(
          WidgetHelpers.testable(
            child: const PairingPage(),
            disableAnimations: true,
          ),
        );
        final toggle = find.widgetWithText(TextButton, 'Use pairing code');
        expect(tester.getBottomLeft(toggle).dy, 844 - 34 - Grid.sm);
        expect(
          tester
              .getBottomLeft(find.byKey(const Key('pairing-buzz-wordmark')))
              .dy,
          lessThan(tester.getTopLeft(toggle).dy - 150),
        );

        final wordmark = find.byKey(const Key('pairing-buzz-wordmark'));
        final originalFrame = tester.getRect(wordmark);
        for (final distance in [-150.0, 150.0]) {
          await tester.drag(find.byType(CustomScrollView), Offset(0, distance));
          await tester.pump(const Duration(milliseconds: 100));
          expect(tester.getRect(wordmark), originalFrame);
          expect(tester.getBottomLeft(toggle).dy, 844 - 34 - Grid.sm);
        }

        await _expandPairingCode(tester);
        tester.view.physicalSize = const Size(360, 560);
        tester.view.viewInsets = const FakeViewPadding(bottom: 240);
        await tester.pump();
        await tester.enterText(find.byType(TextField), 'pairing-code-draft');
        final connect = find.widgetWithText(FilledButton, 'Connect');
        await tester.ensureVisible(connect);
        await tester.pump();
        expect(connect.hitTestable(), findsOneWidget);
        expect(tester.getBottomLeft(connect).dy, lessThanOrEqualTo(560 - 240));
        expect(find.text('pairing-code-draft'), findsOneWidget);
        expect(tester.takeException(), isNull);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.iOS),
    );

    testWidgets('uses dark status-bar icons on the onboarding surface', (
      tester,
    ) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(child: const PairingPage()),
      );

      final overlay = tester.widget<AnnotatedRegion<SystemUiOverlayStyle>>(
        find.byKey(const Key('pairing-onboarding-system-overlay')),
      );

      expect(overlay.value.statusBarIconBrightness, Brightness.dark);
      expect(overlay.value.statusBarColor, Colors.transparent);
    });

    testWidgets(
      'switches welcome artwork and readable controls with appearance',
      (tester) async {
        Future<void> show(Brightness brightness) async {
          await tester.pumpWidget(
            ProviderScope(
              child: MaterialApp(
                theme: brightness == Brightness.dark
                    ? AppTheme.dark()
                    : AppTheme.light(),
                home: const PairingPage(),
              ),
            ),
          );
          await tester.pump(const Duration(milliseconds: 300));
        }

        await show(Brightness.light);
        var decoration =
            tester
                    .widget<DecoratedBox>(
                      find.byKey(const Key('pairing-onboarding-background')),
                    )
                    .decoration
                as BoxDecoration;
        expect(
          (decoration.image!.image as AssetImage).assetName,
          'assets/images/shell-gradient.png',
        );
        expect(decoration.image!.fit, BoxFit.fill);
        await _expandPairingCode(tester);
        await tester.enterText(find.byType(TextField), 'pairing-code-draft');

        await show(Brightness.dark);
        decoration =
            tester
                    .widget<DecoratedBox>(
                      find.byKey(const Key('pairing-onboarding-background')),
                    )
                    .decoration
                as BoxDecoration;
        expect(decoration.image, isNull);
        expect(decoration.color, const Color(0xFF11181D));
        expect(
          tester
              .widget<OnboardingWordmark>(
                find.byKey(const Key('pairing-buzz-wordmark')),
              )
              .color,
          const Color(0xFFE6EDF0),
        );
        expect(
          tester.widget<TextField>(find.byType(TextField)).style!.color,
          const Color(0xFFE6EDF0),
        );
        expect(
          tester
              .widget<TextField>(find.byType(TextField))
              .decoration!
              .fillColor,
          const Color(0xFF233039),
        );
        expect(find.text('pairing-code-draft'), findsOneWidget);
        expect(
          tester
              .widget<AnnotatedRegion<SystemUiOverlayStyle>>(
                find.byKey(const Key('pairing-onboarding-system-overlay')),
              )
              .value
              .statusBarIconBrightness,
          Brightness.light,
        );

        await show(Brightness.light);
        expect(
          tester
              .widget<OnboardingWordmark>(
                find.byKey(const Key('pairing-buzz-wordmark')),
              )
              .color,
          isNull,
        );
        expect(find.text('pairing-code-draft'), findsOneWidget);
        expect(tester.takeException(), isNull);
      },
    );

    testWidgets('uses the onboarding surface for dark-theme SAS verification', (
      tester,
    ) async {
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            pairingProvider.overrideWith(() => _ConfirmingSasPairingNotifier()),
          ],
          child: MaterialApp(theme: AppTheme.dark(), home: const PairingPage()),
        ),
      );

      final overlay = tester.widget<AnnotatedRegion<SystemUiOverlayStyle>>(
        find.byKey(const Key('pairing-sas-system-overlay')),
      );

      expect(overlay.value.statusBarIconBrightness, Brightness.light);
      expect(overlay.value.statusBarColor, Colors.transparent);
      final background = tester.widget<DecoratedBox>(
        find.byKey(const Key('pairing-onboarding-background')),
      );
      final backgroundDecoration = background.decoration as BoxDecoration;
      expect(backgroundDecoration.color, const Color(0xFF11181D));
      expect(backgroundDecoration.image, isNull);
      expect(
        tester.widget<Scaffold>(find.byType(Scaffold)).backgroundColor,
        Colors.transparent,
      );
      expect(find.text('Confirm desktop code'), findsOneWidget);
      expect(
        find.text(
          'Make sure the six-digit code matches on both devices. Your Buzz identity will transfer to this device. Only continue if you started this pairing from your desktop.',
        ),
        findsOneWidget,
      );
      expect(find.text('Does your desktop app show this code?'), findsNothing);
    });

    testWidgets('uses Cancel as the only visible SAS exit', (tester) async {
      final notifier = _ConfirmingSasPairingNotifier();
      await tester.pumpWidget(
        ProviderScope(
          overrides: [pairingProvider.overrideWith(() => notifier)],
          child: MaterialApp(
            theme: AppTheme.dark(),
            home: const PairingPage(addingCommunity: true),
          ),
        ),
      );

      expect(find.byType(AppBar), findsNothing);
      expect(find.text('Add Community'), findsNothing);
      expect(find.byIcon(LucideIcons.arrowLeft), findsNothing);
      expect(find.byKey(const Key('pairing-pop-scope')), findsOneWidget);

      await tester.tap(find.widgetWithText(TextButton, 'Cancel'));
      expect(notifier.denied, isTrue);
    });

    testWidgets('keeps the add-community header outside SAS', (tester) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(child: const PairingPage(addingCommunity: true)),
      );

      expect(find.byType(AppBar), findsOneWidget);
      expect(find.text('Add Community'), findsOneWidget);
      expect(find.byIcon(LucideIcons.arrowLeft), findsOneWidget);
    });

    testWidgets('uses the native navigation bar on iOS', (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);

      await tester.pumpWidget(
        WidgetHelpers.testable(child: const PairingPage(addingCommunity: true)),
      );

      final nativeBar = tester.widget<UiKitView>(find.byType(UiKitView));
      expect(nativeBar.viewType, 'buzz/ios_navigation_bar');
      expect(nativeBar.creationParams, containsPair('title', 'Add Community'));
      debugDefaultTargetPlatformOverride = null;
    });

    testWidgets('reveals pairing code field and connect action', (
      tester,
    ) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(child: const PairingPage()),
      );

      await _expandPairingCode(tester);

      expect(find.text('Hide pairing code'), findsOneWidget);
      expect(find.text('Connect'), findsOneWidget);
      expect(find.byType(TextField), findsOneWidget);
    });

    testWidgets('connect button is below text field, not beside it', (
      tester,
    ) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(child: const PairingPage()),
      );
      await _expandPairingCode(tester);

      final textField = tester.getBottomLeft(find.byType(TextField));
      final connectButton = tester.getTopLeft(
        find.widgetWithText(FilledButton, 'Connect'),
      );

      // The connect button should be below the text field.
      expect(connectButton.dy, greaterThan(textField.dy));
    });

    testWidgets('connect button is full width', (tester) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(child: const PairingPage()),
      );
      await _expandPairingCode(tester);

      final connectButton = tester.getSize(
        find.widgetWithText(FilledButton, 'Connect'),
      );
      final textField = tester.getSize(find.byType(TextField));

      // Button width should be close to the text field width (both full-width).
      expect(connectButton.width, closeTo(textField.width, 2.0));
    });

    testWidgets('shows error container when pairing fails', (tester) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            pairingProvider.overrideWith(
              () => _ErrorPairingNotifier('Invalid pairing code: bad input'),
            ),
          ],
          child: const PairingPage(),
        ),
      );
      await tester.pump();

      expect(find.text('Invalid pairing code: bad input'), findsOneWidget);
    });

    testWidgets('shows spinner when connecting', (tester) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            pairingProvider.overrideWith(() => _ConnectingPairingNotifier()),
          ],
          child: const PairingPage(),
        ),
      );
      await tester.pump();

      expect(find.byType(BuzzLoadingIndicator), findsOneWidget);
      // Connect text should be replaced by spinner.
      expect(find.text('Connect'), findsNothing);
    });

    testWidgets('pairing actions are disabled when connecting', (tester) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            pairingProvider.overrideWith(() => _ConnectingPairingNotifier()),
          ],
          child: const PairingPage(),
        ),
      );
      await tester.pump();

      final scanButton = tester.widget<FilledButton>(find.byType(FilledButton));
      final pairingCodeButton = tester.widget<TextButton>(
        find.widgetWithText(TextButton, 'Use pairing code'),
      );

      expect(scanButton.onPressed, isNull);
      expect(pairingCodeButton.onPressed, isNull);
    });

    testWidgets('recovery entry rejects ordinary nostrpair codes', (
      tester,
    ) async {
      final notifier = _RecordingPairingNotifier();
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [pairingProvider.overrideWith(() => notifier)],
          child: const PairingPage(
            addingCommunity: true,
            identityRecoveryOnly: true,
          ),
        ),
      );

      await _expandPairingCode(tester);
      await tester.enterText(find.byType(TextField), 'nostrpair://ordinary');
      await tester.ensureVisible(find.text('Connect'));
      await tester.pump();
      await tester.tap(find.text('Connect'));
      await tester.pump();

      expect(find.text('Scan a desktop recovery code.'), findsOneWidget);
      expect(notifier.pairedCodes, isEmpty);
    });

    testWidgets('recovery entry accepts mode=recover codes', (tester) async {
      final notifier = _RecordingPairingNotifier();
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [pairingProvider.overrideWith(() => notifier)],
          child: const PairingPage(
            addingCommunity: true,
            identityRecoveryOnly: true,
          ),
        ),
      );

      await _expandPairingCode(tester);
      const code = 'nostrpair://desktop?mode=recover';
      await tester.enterText(find.byType(TextField), code);
      await tester.ensureVisible(find.text('Connect'));
      await tester.pump();
      await tester.tap(find.text('Connect'));
      await tester.pump();

      expect(notifier.pairedCodes, [code]);
    });

    testWidgets('new identity import offers protection checked by default', (
      tester,
    ) async {
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            pairingProvider.overrideWith(() => _ConfirmingSasPairingNotifier()),
          ],
          child: MaterialApp(theme: AppTheme.dark(), home: const PairingPage()),
        ),
      );

      final checkbox = tester.widget<CheckboxListTile>(
        find.byKey(const Key('protect-sensitive-actions-checkbox')),
      );
      expect(checkbox.value, isTrue);
      expect(find.text('Use biometrics'), findsOneWidget);
      expect(find.text('For secure actions'), findsOneWidget);
    });

    testWidgets('uses the native Face ID label on iOS', (tester) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      try {
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              pairingProvider.overrideWith(
                () => _ConfirmingSasPairingNotifier(),
              ),
              enrolledBiometricsProvider.overrideWith(
                (_) async => const [BiometricType.face],
              ),
            ],
            child: MaterialApp(
              theme: AppTheme.dark(),
              home: const PairingPage(),
            ),
          ),
        );
        await tester.pump();

        expect(find.text('Use Face ID'), findsOneWidget);
        expect(find.text('Use biometrics'), findsNothing);
      } finally {
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('uses the native Touch ID label on iOS', (tester) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      try {
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              pairingProvider.overrideWith(
                () => _ConfirmingSasPairingNotifier(),
              ),
              enrolledBiometricsProvider.overrideWith(
                (_) async => const [BiometricType.fingerprint],
              ),
            ],
            child: MaterialApp(
              theme: AppTheme.dark(),
              home: const PairingPage(),
            ),
          ),
        );
        await tester.pump();

        expect(find.text('Use Touch ID'), findsOneWidget);
        expect(find.text('Use Face ID'), findsNothing);
      } finally {
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('desktop recovery does not show protection checkbox', (
      tester,
    ) async {
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            pairingProvider.overrideWith(
              () => _ConfirmingSasPairingNotifier(sendsIdentityToDesktop: true),
            ),
          ],
          child: MaterialApp(theme: AppTheme.dark(), home: const PairingPage()),
        ),
      );

      expect(
        find.byKey(const Key('protect-sensitive-actions-checkbox')),
        findsNothing,
      );
    });

    testWidgets('recovery SAS puts permanent desktop access in the subtitle', (
      tester,
    ) async {
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            pairingProvider.overrideWith(
              () => _ConfirmingSasPairingNotifier(sendsIdentityToDesktop: true),
            ),
          ],
          child: MaterialApp(theme: AppTheme.dark(), home: const PairingPage()),
        ),
      );

      expect(find.textContaining('full Buzz identity'), findsOneWidget);
      expect(find.textContaining('permanent access'), findsOneWidget);
      expect(find.textContaining('started this recovery'), findsOneWidget);
      expect(find.text('Codes match'), findsOneWidget);
    });

    testWidgets('matches the onboarding visual system and SAS action layout', (
      tester,
    ) async {
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            pairingProvider.overrideWith(() => _ConfirmingSasPairingNotifier()),
          ],
          child: MaterialApp(theme: AppTheme.dark(), home: const PairingPage()),
        ),
      );

      expect(find.byIcon(LucideIcons.shieldCheck), findsNothing);
      expect(find.text('Confirm desktop code'), findsOneWidget);
      expect(
        find.text(
          'Make sure the six-digit code matches on both devices. Your Buzz identity will transfer to this device. Only continue if you started this pairing from your desktop.',
        ),
        findsOneWidget,
      );
      expect(find.text('Does your desktop app show this code?'), findsNothing);

      final digitFinders = [
        for (var index = 1; index <= 6; index++)
          find.byKey(Key('pairing-sas-code-digit-$index')),
      ];
      for (final digitFinder in digitFinders) {
        expect(tester.getSize(digitFinder).width, 54);
        expect(
          tester.widget<Container>(digitFinder).padding,
          const EdgeInsets.symmetric(vertical: Grid.xs),
        );
      }

      const onboardingInk = Color(0xFFE6EDF0);
      const onboardingMutedInk = Color(0xFFAAB8C0);
      const onboardingCtaLabel = Color(0xFF172229);
      final theme = AppTheme.dark();
      final protectionTile = tester.widget<CheckboxListTile>(
        find.byKey(const Key('protect-sensitive-actions-checkbox')),
      );
      expect(protectionTile.activeColor, onboardingInk);
      expect(protectionTile.checkColor, onboardingCtaLabel);
      expect(protectionTile.side?.color, onboardingInk);
      expect((protectionTile.title as Text).style?.color, onboardingInk);
      expect(
        (protectionTile.subtitle as Text).style?.color,
        onboardingMutedInk,
      );
      final firstDigitContainer = tester.widget<Container>(digitFinders.first);
      final firstDigitDecoration =
          firstDigitContainer.decoration! as BoxDecoration;
      expect(firstDigitDecoration.color, const Color(0xFF233039));
      expect(
        (firstDigitDecoration.border! as Border).top.color,
        theme.colorScheme.primary.withValues(alpha: 0.15),
      );
      final firstDigitText = tester.widget<Text>(
        find.descendant(of: digitFinders.first, matching: find.text('1')),
      );
      expect(firstDigitText.style?.fontFamily, 'Inter');
      expect(
        firstDigitText.style?.fontSize,
        theme.textTheme.displaySmall?.fontSize,
      );
      expect(firstDigitText.style?.fontSize, greaterThanOrEqualTo(36));
      expect(firstDigitText.style?.fontWeight, FontWeight.w600);
      expect(firstDigitText.style?.fontFeatures, isNull);
      expect(firstDigitText.style?.color, onboardingInk);

      final firstDigit = tester.getTopLeft(digitFinders[0]);
      final secondDigit = tester.getTopLeft(digitFinders[1]);
      final thirdDigit = tester.getTopLeft(digitFinders[2]);
      final fourthDigit = tester.getTopLeft(digitFinders[3]);
      expect(secondDigit.dx - firstDigit.dx, 60);
      expect(fourthDigit.dx - thirdDigit.dx, 68);

      final confirmFinder = find.widgetWithText(FilledButton, 'Codes match');
      final cancelFinder = find.widgetWithText(TextButton, 'Cancel');
      final confirmButton = tester.widget<FilledButton>(confirmFinder);
      final cancelButton = tester.widget<TextButton>(cancelFinder);
      expect(
        confirmButton.style?.backgroundColor?.resolve(<WidgetState>{}),
        onboardingInk,
      );
      expect(
        confirmButton.style?.foregroundColor?.resolve(<WidgetState>{}),
        onboardingCtaLabel,
      );
      expect(
        confirmButton.style?.shape?.resolve(<WidgetState>{}),
        isA<StadiumBorder>(),
      );
      expect(
        cancelButton.style?.backgroundColor?.resolve(<WidgetState>{}),
        onboardingInk.withValues(alpha: 0.1),
      );
      expect(
        cancelButton.style?.foregroundColor?.resolve(<WidgetState>{}),
        onboardingInk,
      );
      expect(
        cancelButton.style?.shape?.resolve(<WidgetState>{}),
        isA<StadiumBorder>(),
      );
      final confirmTopLeft = tester.getTopLeft(confirmFinder);
      final cancelTopLeft = tester.getTopLeft(cancelFinder);
      final scaffoldWidth = tester.getSize(find.byType(Scaffold)).width;
      expect(confirmTopLeft.dy, lessThan(cancelTopLeft.dy));
      expect(confirmTopLeft.dx, cancelTopLeft.dx);
      expect(confirmTopLeft.dx, Grid.sm);
      expect(tester.getSize(confirmFinder).width, scaffoldWidth - Grid.sm * 2);
      expect(tester.getSize(cancelFinder).width, scaffoldWidth - Grid.sm * 2);
      expect(tester.getSize(confirmFinder).height, 48);
      expect(tester.getSize(cancelFinder).height, 48);
      expect(
        find.textContaining(
          'Only continue if you started this pairing from your desktop.',
        ),
        findsOneWidget,
      );
      expect(
        tester.getBottomLeft(find.byType(Scaffold)).dy -
            tester.getBottomLeft(cancelFinder).dy,
        Grid.sm,
      );
    });

    for (final brightness in Brightness.values) {
      testWidgets('uses accessible SAS error contrast in ${brightness.name}', (
        tester,
      ) async {
        const errorMessage =
            'Identity confirmation failed. Nothing transferred.';
        final isDark = brightness == Brightness.dark;
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              pairingProvider.overrideWith(
                () => _ConfirmingSasPairingNotifier(errorMessage: errorMessage),
              ),
            ],
            child: MaterialApp(
              theme: isDark ? AppTheme.dark() : AppTheme.light(),
              home: const PairingPage(),
            ),
          ),
        );
        await tester.pumpAndSettle();

        final errorFinder = find.text(errorMessage);
        expect(Theme.of(tester.element(errorFinder)).brightness, brightness);
        final errorInk = tester.widget<Text>(errorFinder).style!.color!;
        expect(
          errorInk,
          isDark ? const Color(0xFFFFAAA0) : const Color(0xFF7A1025),
        );

        final backgroundFinder = find.byKey(
          const Key('pairing-onboarding-background'),
        );
        final background = tester.widget<DecoratedBox>(backgroundFinder);
        final decoration = background.decoration as BoxDecoration;
        expect(
          decoration.color,
          isDark ? const Color(0xFF11181D) : const Color(0xFFE7F0EF),
        );
        if (isDark) {
          expect(decoration.image, isNull);
        } else {
          expect(
            (decoration.image!.image as AssetImage).assetName,
            'assets/images/shell-gradient.png',
          );
          await tester.runAsync(
            () => precacheImage(
              decoration.image!.image,
              tester.element(backgroundFinder),
            ),
          );
        }
        final size = tester.getSize(backgroundFinder);
        final errorRect = tester
            .getRect(errorFinder)
            .shift(-tester.getTopLeft(backgroundFinder));

        // Render the production decoration and painter without foreground
        // content, so sampled pixels are the actual surface behind the error.
        const captureKey = Key('sas-error-background-capture');
        await tester.pumpWidget(
          Directionality(
            textDirection: TextDirection.ltr,
            child: RepaintBoundary(
              key: captureKey,
              child: SizedBox.fromSize(
                size: size,
                child: DecoratedBox(
                  decoration: decoration,
                  child: CustomPaint(
                    painter: (background.child! as CustomPaint).painter,
                  ),
                ),
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        final boundary = tester.renderObject<RenderRepaintBoundary>(
          find.byKey(captureKey),
        );
        await tester.runAsync(() async {
          final image = await boundary.toImage(pixelRatio: 1);
          try {
            final pixels = (await image.toByteData(
              format: ui.ImageByteFormat.rawRgba,
            ))!;
            for (var y = errorRect.top.ceil(); y < errorRect.bottom; y += 4) {
              for (var x = errorRect.left.ceil(); x < errorRect.right; x += 4) {
                final offset = (y * image.width + x) * 4;
                expect(pixels.getUint8(offset + 3), 255);
                final surface = Color.fromARGB(
                  255,
                  pixels.getUint8(offset),
                  pixels.getUint8(offset + 1),
                  pixels.getUint8(offset + 2),
                );
                expect(
                  _contrastRatio(errorInk, surface),
                  greaterThanOrEqualTo(4.5),
                  reason: '${brightness.name} error contrast at ($x, $y)',
                );
              }
            }
          } finally {
            image.dispose();
          }
        });
        expect(tester.takeException(), isNull);
      });
    }

    testWidgets('keeps SAS actions above the keyboard on small screens', (
      tester,
    ) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(360, 560);
      tester.view.viewInsets = const FakeViewPadding(bottom: 200);
      addTearDown(tester.view.reset);

      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            pairingProvider.overrideWith(() => _ConfirmingSasPairingNotifier()),
          ],
          child: MaterialApp(theme: AppTheme.dark(), home: const PairingPage()),
        ),
      );

      expect(tester.takeException(), isNull);
      expect(find.byType(SingleChildScrollView), findsOneWidget);
      final cancelFinder = find.widgetWithText(TextButton, 'Cancel');
      expect(tester.getBottomLeft(cancelFinder).dy, 560 - 200 - Grid.sm);

      await tester.drag(
        find.byType(SingleChildScrollView),
        const Offset(0, -100),
      );
      await tester.pump();
      expect(tester.takeException(), isNull);
      expect(find.text('Confirm desktop code'), findsOneWidget);
      expect(find.textContaining('matches on both devices'), findsOneWidget);
      expect(
        find.textContaining('Buzz identity will transfer'),
        findsOneWidget,
      );
      expect(find.text('Codes match'), findsOneWidget);
    });
  });
}

double _contrastRatio(Color foreground, Color background) {
  final foregroundLuminance = foreground.computeLuminance();
  final backgroundLuminance = background.computeLuminance();
  final lighter = foregroundLuminance > backgroundLuminance
      ? foregroundLuminance
      : backgroundLuminance;
  final darker = foregroundLuminance > backgroundLuminance
      ? backgroundLuminance
      : foregroundLuminance;
  return (lighter + 0.05) / (darker + 0.05);
}

Future<void> _expandPairingCode(WidgetTester tester) async {
  await tester.ensureVisible(find.text('Use pairing code'));
  await tester.pump();
  await tester.tap(find.text('Use pairing code'));
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 300));
}

class _ErrorPairingNotifier extends Notifier<PairingState>
    implements PairingNotifier {
  final String error;
  _ErrorPairingNotifier(this.error);

  @override
  PairingState build() =>
      PairingState(status: PairingStatus.error, errorMessage: error);

  @override
  Future<bool> authorizeIdentityExport({required Community community}) async =>
      true;

  @override
  Future<void> pair(String rawInput) async {}

  @override
  void reset() {}

  @override
  void confirmSas() {}

  @override
  void setProtectSensitiveActions(bool value) {}

  @override
  void denySas() {}
}

class _ConnectingPairingNotifier extends Notifier<PairingState>
    implements PairingNotifier {
  @override
  PairingState build() => const PairingState(status: PairingStatus.connecting);

  @override
  Future<bool> authorizeIdentityExport({required Community community}) async =>
      true;

  @override
  Future<void> pair(String rawInput) async {}

  @override
  void reset() {}

  @override
  void confirmSas() {}

  @override
  void setProtectSensitiveActions(bool value) {}

  @override
  void denySas() {}
}

class _RecordingPairingNotifier extends Notifier<PairingState>
    implements PairingNotifier {
  final pairedCodes = <String>[];

  @override
  PairingState build() => const PairingState();

  @override
  Future<bool> authorizeIdentityExport({required Community community}) async =>
      true;

  @override
  Future<void> pair(String rawInput) async => pairedCodes.add(rawInput);

  @override
  void reset() {}

  @override
  void confirmSas() {}

  @override
  void setProtectSensitiveActions(bool value) {}

  @override
  void denySas() {}
}

class _ConfirmingSasPairingNotifier extends Notifier<PairingState>
    implements PairingNotifier {
  _ConfirmingSasPairingNotifier({
    this.sendsIdentityToDesktop = false,
    this.errorMessage,
  });

  final bool sendsIdentityToDesktop;
  final String? errorMessage;
  bool denied = false;

  @override
  PairingState build() => PairingState(
    status: PairingStatus.confirmingSas,
    sasCode: '123456',
    sendsIdentityToDesktop: sendsIdentityToDesktop,
    errorMessage: errorMessage,
  );

  @override
  Future<bool> authorizeIdentityExport({required Community community}) async =>
      true;

  @override
  Future<void> pair(String rawInput) async {}

  @override
  void reset() {}

  @override
  void confirmSas() {}

  @override
  void setProtectSensitiveActions(bool value) {}

  @override
  void denySas() => denied = true;
}
