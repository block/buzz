import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:local_auth/local_auth.dart';
import 'package:buzz/features/pairing/pairing_page.dart';
import 'package:buzz/features/pairing/pairing_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/security/sensitive_action_authorizer.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/buzz_loading_indicator.dart';
import 'package:buzz/shared/widgets/tappable_flapping_bee.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  group('PairingPage', () {
    testWidgets('renders branding and progressive pairing actions', (
      tester,
    ) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(child: const PairingPage()),
      );

      expect(find.byType(TappableFlappingBee), findsOneWidget);
      expect(find.text('Welcome to Buzz'), findsOneWidget);
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

      expect(scanButton.width, lessThan(440));
      expect(pairingCodeButton.width, lessThan(440));
      expect(find.byType(OutlinedButton), findsNothing);
    });

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

    testWidgets('uses light status-bar icons for dark-theme SAS verification', (
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
      expect(find.text('Verify Security Code'), findsOneWidget);
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

    testWidgets('recovery SAS warns about permanent desktop access', (
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
      expect(find.text('Codes Match'), findsOneWidget);
    });
  });
}

Future<void> _expandPairingCode(WidgetTester tester) async {
  await tester.tap(find.text('Use pairing code'));
  await tester.pumpAndSettle();
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
  _ConfirmingSasPairingNotifier({this.sendsIdentityToDesktop = false});

  final bool sendsIdentityToDesktop;

  @override
  PairingState build() => PairingState(
    status: PairingStatus.confirmingSas,
    sasCode: '123456',
    sendsIdentityToDesktop: sendsIdentityToDesktop,
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
  void denySas() {}
}
