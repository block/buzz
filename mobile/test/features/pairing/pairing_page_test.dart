import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/pairing/pairing_page.dart';
import 'package:buzz/features/pairing/pairing_provider.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  group('PairingPage', () {
    testWidgets('renders branding, scan button, divider, and text field', (
      tester,
    ) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(child: const PairingPage()),
      );

      expect(
        find.image(const AssetImage('assets/images/buzz-icon.png')),
        findsOneWidget,
      );
      expect(find.text('Welcome to Buzz'), findsOneWidget);
      expect(find.text('Scan QR Code'), findsOneWidget);
      expect(find.text('or paste pairing code'), findsOneWidget);
      expect(find.text('Connect'), findsOneWidget);
      expect(find.byType(TextField), findsOneWidget);
    });

    testWidgets('connect button is below text field, not beside it', (
      tester,
    ) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(child: const PairingPage()),
      );

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

      expect(find.byType(CircularProgressIndicator), findsOneWidget);
      // Connect text should be replaced by spinner.
      expect(find.text('Connect'), findsNothing);
    });

    testWidgets('text field and buttons disabled when connecting', (
      tester,
    ) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            pairingProvider.overrideWith(() => _ConnectingPairingNotifier()),
          ],
          child: const PairingPage(),
        ),
      );
      await tester.pump();

      final textField = tester.widget<TextField>(find.byType(TextField));
      expect(textField.enabled, isFalse);
    });
  });
}

class _ErrorPairingNotifier extends Notifier<PairingState>
    implements PairingNotifier {
  final String error;
  _ErrorPairingNotifier(this.error);

  @override
  PairingState build() =>
      PairingState(status: PairingStatus.error, errorMessage: error);

  @override
  Future<void> pair(String rawInput) async {}

  @override
  void reset() {}

  @override
  void confirmSas() {}

  @override
  void denySas() {}
}

class _ConnectingPairingNotifier extends Notifier<PairingState>
    implements PairingNotifier {
  @override
  PairingState build() => const PairingState(status: PairingStatus.connecting);

  @override
  Future<void> pair(String rawInput) async {}

  @override
  void reset() {}

  @override
  void confirmSas() {}

  @override
  void denySas() {}
}
