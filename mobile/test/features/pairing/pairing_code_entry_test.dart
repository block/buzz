import 'dart:async';
import 'package:buzz/features/pairing/pairing_page.dart';
import 'package:buzz/features/pairing/pairing_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _EntryNotifier extends PairingNotifier {
  _EntryNotifier({
    this.code = '123456',
    this.recovery = false,
    this.remote = false,
  });
  final bool remote;
  final submissions = <String>[];
  Completer<bool>? verification;
  @override
  Future<bool> verifyDesktopCode(String value) {
    submissions.add(value);
    verification = Completer<bool>();
    return verification!.future;
  }

  void completePairing() =>
      state = state.copyWith(status: PairingStatus.success);
  final String code;
  final bool recovery;
  int confirmations = 0;
  bool? chosenProtection;

  @override
  PairingState build() => PairingState(
    status: PairingStatus.confirmingSas,
    sasCode: code,
    sendsIdentityToDesktop: recovery,
    requiresDesktopCode: remote,
  );

  @override
  void confirmSas() {
    confirmations++;
    state = state.copyWith(userConfirmedSas: true);
  }

  @override
  void setProtectSensitiveActions(bool value) {
    chosenProtection = value;
    super.setProtectSensitiveActions(value);
  }

  void biometricCancelled() {
    state = state.copyWith(
      userConfirmedSas: false,
      errorMessage: 'Biometric confirmation cancelled. Try again or skip.',
    );
  }
}

void main() {
  final field = find.byKey(const Key('pairing-code-input'));

  Future<_EntryNotifier> showEntry(
    WidgetTester tester, {
    bool reducedMotion = false,
    String code = '123456',
    bool recovery = false,
    bool remote = false,
  }) async {
    final notifier = _EntryNotifier(
      code: code,
      recovery: recovery,
      remote: remote,
    );
    await tester.pumpWidget(
      ProviderScope(
        overrides: [pairingProvider.overrideWith(() => notifier)],
        child: MaterialApp(
          theme: AppTheme.light(),
          builder: (context, child) => MediaQuery(
            data: MediaQuery.of(
              context,
            ).copyWith(disableAnimations: reducedMotion),
            child: child!,
          ),
          home: const PairingPage(),
        ),
      ),
    );
    return notifier;
  }

  testWidgets(
    'remote verification waits for desktop and does not trust the derived SAS',
    (tester) async {
      final notifier = await showEntry(tester, remote: true);
      await tester.enterText(field, '123456');
      await tester.pump();
      expect(find.text('Protect your identity'), findsNothing);
      expect(notifier.submissions, ['123456']);
      notifier.verification!.complete(false);
      await tester.pumpAndSettle();
      expect(find.text('Protect your identity'), findsNothing);
      tester.testTextInput.updateEditingValue(
        const TextEditingValue(text: '1234567'),
      );
      await tester.pump();
      expect(notifier.submissions, ['123456']);
      await tester.enterText(field, '654321');
      await tester.pump();
      notifier.verification!.complete(true);
      await tester.pumpAndSettle();
      expect(find.text('Protect your identity'), findsOneWidget);
      expect(notifier.confirmations, 0);
      await tester.tap(find.text('Skip'));
      await tester.pump();
      expect(notifier.confirmations, 1);
    },
  );

  testWidgets(
    'successful pushed pairing resets after dismissal and can reopen',
    (tester) async {
      final notifier = _EntryNotifier();
      await tester.pumpWidget(
        ProviderScope(
          overrides: [pairingProvider.overrideWith(() => notifier)],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Builder(
              builder: (context) => Scaffold(
                body: TextButton(
                  onPressed: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) => const PairingPage(addingCommunity: true),
                    ),
                  ),
                  child: const Text('Open pairing'),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open pairing'));
      await tester.pumpAndSettle();
      notifier.completePairing();
      await tester.pump();
      await tester.pumpAndSettle();
      expect(find.byType(PairingPage), findsNothing);
      await tester.tap(find.text('Open pairing'));
      await tester.pumpAndSettle();
      expect(find.text('Scan a QR code'), findsOneWidget);
      expect(find.byKey(const Key('pairing-community-loading')), findsNothing);
    },
  );

  testWidgets(
    'starts empty with no extra confirmation or biometrics controls',
    (tester) async {
      await showEntry(tester);
      expect(find.text('Enter pairing code'), findsOneWidget);
      expect(tester.widget<TextField>(field).controller!.text, isEmpty);
      expect(find.text('123456'), findsNothing);
      expect(find.text('123 456'), findsNothing);
      expect(find.text('Continue'), findsNothing);
      expect(find.text('Use biometrics'), findsNothing);
      for (var index = 0; index < 6; index++) {
        expect(find.byKey(Key('pairing-code-cell-$index')), findsOneWidget);
      }
    },
  );

  testWidgets('wrong code shakes only the digit boxes with one haptic', (
    tester,
  ) async {
    final notifier = await showEntry(tester);
    final haptics = <Object?>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(const MethodChannel('buzz/haptics'), (
          call,
        ) async {
          if (call.method == 'error') {
            haptics.add(call.method);
          }
          return null;
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('buzz/haptics'), null),
    );
    await tester.enterText(field, '999999');
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 40));
    final shake = tester.widget<Transform>(
      find.byKey(const Key('pairing-error-shake')),
    );
    expect(shake.transform.storage[12].abs(), greaterThan(0));
    expect(
      find.ancestor(
        of: find.text('Enter pairing code'),
        matching: find.byKey(const Key('pairing-error-shake')),
      ),
      findsNothing,
    );
    final cell = tester.widget<AnimatedContainer>(
      find.byKey(const Key('pairing-code-cell-0')),
    );
    final errorColor = Theme.of(tester.element(field)).colorScheme.error;
    expect(
      ((cell.decoration as BoxDecoration).border as Border).top.color,
      errorColor,
    );
    expect(
      tester
          .widget<Text>(find.text('Check the desktop code and try again.'))
          .style!
          .color,
      tester.widget<Text>(find.text('Enter pairing code')).style!.color,
    );
    expect(haptics, ['error']);
    expect(find.text('Check the desktop code and try again.'), findsOneWidget);
    expect(notifier.confirmations, 0);
    expect(find.text('Protect your identity'), findsNothing);
  });

  testWidgets(
    'extra digits repeat error feedback even when the code stays full',
    (tester) async {
      await showEntry(tester);
      var errorHaptics = 0;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('buzz/haptics'), (
            call,
          ) async {
            if (call.method == 'error') errorHaptics++;
            return null;
          });
      addTearDown(
        () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(
              const MethodChannel('buzz/haptics'),
              null,
            ),
      );
      final heading = find.text('Enter pairing code');
      final titlePosition = tester.getTopLeft(heading);
      await tester.enterText(field, '999999');
      await tester.pumpAndSettle();
      expect(errorHaptics, 1);
      for (final digit in ['1', '2', '3']) {
        await tester.enterText(field, '999999$digit');
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 40));
        final shake = tester.widget<Transform>(
          find.byKey(const Key('pairing-error-shake')),
        );
        expect(shake.transform.storage[12].abs(), greaterThan(0));
        expect(tester.widget<TextField>(field).controller!.text, '999999');
        expect(tester.getTopLeft(heading), titlePosition);
      }
      expect(errorHaptics, 4);
      await tester.enterText(field, '99999');
      await tester.pumpAndSettle();
      expect(
        errorHaptics,
        4,
        reason: 'Deleting to correct the code is not an error',
      );
      await tester.enterText(field, '123456');
      await tester.pumpAndSettle();
      expect(errorHaptics, 4);
      expect(find.text('Protect your identity'), findsOneWidget);
    },
  );

  testWidgets('reference shake keyframes and error fade preserve layout', (
    tester,
  ) async {
    await showEntry(tester);
    final heading = find.text('Enter pairing code');
    final initialPosition = tester.getTopLeft(heading);
    final initialY = initialPosition.dy;
    final error = find.byKey(const Key('pairing-code-error'));
    expect(tester.widget<AnimatedOpacity>(error).opacity, 0);
    await tester.enterText(field, '999999');
    await tester.pump();
    expect(tester.getTopLeft(heading).dy, initialY);
    expect(
      tester.widget<AnimatedOpacity>(error).duration,
      const Duration(milliseconds: 280),
    );
    for (final (milliseconds, expectedX) in [
      (80, 6.0),
      (80, -6.0),
      (60, 4.0),
      (60, 0.0),
    ]) {
      await tester.pump(Duration(milliseconds: milliseconds));
      final transform = tester.widget<Transform>(
        find.byKey(const Key('pairing-error-shake')),
      );
      expect(transform.transform.storage[12], closeTo(expectedX, 0.01));
      expect(tester.getTopLeft(heading), initialPosition);
      expect(
        find.ancestor(
          of: field,
          matching: find.byKey(const Key('pairing-error-shake')),
        ),
        findsOneWidget,
      );
    }
    expect(
      tester
          .widget<AnimatedContainer>(
            find.byKey(const Key('pairing-code-cell-0')),
          )
          .duration,
      const Duration(milliseconds: 280),
    );
    await tester.enterText(field, '99999');
    await tester.pump();
    expect(tester.widget<AnimatedOpacity>(error).opacity, 0);
    expect(tester.getTopLeft(heading).dy, initialY);
    await tester.pump(const Duration(milliseconds: 280));
    final fade = tester.widget<FadeTransition>(
      find.descendant(of: error, matching: find.byType(FadeTransition)).first,
    );
    expect(fade.opacity.value, 0);
  });

  testWidgets(
    'correct code advances automatically and Use biometrics confirms once',
    (tester) async {
      final notifier = await showEntry(tester);
      await tester.enterText(field, '123456');
      await tester.pump();
      expect(field, findsNothing);
      expect(find.text('Protect your identity'), findsOneWidget);
      expect(
        find.textContaining('before sending your Buzz identity'),
        findsOneWidget,
      );
      expect(notifier.confirmations, 0);
      final useBiometrics = find.widgetWithText(FilledButton, 'Use biometrics');
      await tester.tap(useBiometrics);
      await tester.tap(useBiometrics);
      await tester.pump();
      expect(notifier.chosenProtection, isTrue);
      expect(notifier.confirmations, 1);
      expect(find.text('Confirmed — waiting for desktop'), findsOneWidget);
    },
  );

  testWidgets('Skip records disabled protection before confirming', (
    tester,
  ) async {
    final notifier = await showEntry(tester);
    await tester.enterText(field, '123456');
    await tester.pump();
    await tester.tap(find.text('Skip'));
    await tester.pump();
    expect(notifier.chosenProtection, isFalse);
    expect(notifier.confirmations, 1);
  });

  testWidgets(
    'cancelled biometrics lets the user retry or skip without retyping',
    (tester) async {
      final notifier = await showEntry(tester);
      await tester.enterText(field, '123456');
      await tester.pump();
      await tester.tap(find.text('Use biometrics'));
      await tester.pump();
      notifier.biometricCancelled();
      await tester.pump();
      expect(find.text('Protect your identity'), findsOneWidget);
      expect(
        find.textContaining('Biometric confirmation cancelled'),
        findsOneWidget,
      );
      await tester.tap(find.text('Skip'));
      await tester.pump();
      expect(notifier.chosenProtection, isFalse);
      expect(notifier.confirmations, 2);
    },
  );

  testWidgets(
    'desktop recovery confirms matching input without a protection step',
    (tester) async {
      final notifier = await showEntry(tester, recovery: true);
      await tester.enterText(field, '123456');
      await tester.pump();
      expect(notifier.confirmations, 1);
      expect(find.text('Protect your identity'), findsNothing);
      expect(find.text('Code confirmed'), findsOneWidget);
    },
  );

  testWidgets('desktop recovery can retry cancelled authorization', (
    tester,
  ) async {
    final notifier = await showEntry(tester, recovery: true);
    await tester.enterText(field, '123456');
    await tester.pump();
    notifier.biometricCancelled();
    await tester.pump();
    await tester.tap(find.text('Try again'));
    await tester.pump();
    expect(notifier.confirmations, 2);
    expect(notifier.chosenProtection, isNull);
  });

  testWidgets('paste filters non-digits, limits length and preserves zeros', (
    tester,
  ) async {
    final notifier = await showEntry(tester, code: '012345');
    await tester.enterText(field, '01 23-45abc67');
    await tester.pump();
    expect(find.text('Protect your identity'), findsOneWidget);
    expect(notifier.confirmations, 0);
  });

  testWidgets(
    'tapping a cell selects its digit; deleting and correcting advances',
    (tester) async {
      await showEntry(tester);
      await tester.enterText(field, '123956');
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('pairing-code-cell-3')));
      await tester.pump();
      final controller = tester.widget<TextField>(field).controller!;
      expect(
        controller.selection,
        const TextSelection(baseOffset: 3, extentOffset: 4),
      );
      tester.testTextInput.updateEditingValue(
        const TextEditingValue(
          text: '12356',
          selection: TextSelection.collapsed(offset: 3),
        ),
      );
      await tester.pump();
      expect(controller.text, '12356');
      expect(
        tester
            .widget<AnimatedOpacity>(
              find.byKey(const Key('pairing-code-error')),
            )
            .opacity,
        0,
      );
      tester.testTextInput.updateEditingValue(
        const TextEditingValue(
          text: '123456',
          selection: TextSelection.collapsed(offset: 4),
        ),
      );
      await tester.pump();
      expect(field, findsNothing);
      expect(find.text('Protect your identity'), findsOneWidget);
    },
  );

  testWidgets('reduced motion removes shaking but keeps the error message', (
    tester,
  ) async {
    await showEntry(tester, reducedMotion: true);
    await tester.enterText(field, '999999');
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 40));
    final shake = tester.widget<Transform>(
      find.byKey(const Key('pairing-error-shake')),
    );
    expect(shake.transform.storage[12], 0);
    final cell = find.byKey(const Key('pairing-code-cell-0'));
    expect(tester.widget<AnimatedContainer>(cell).duration, Duration.zero);
    expect(find.text('Check the desktop code and try again.'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('numeric entry is exposed as one labelled accessible field', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    await showEntry(tester);
    expect(find.bySemanticsLabel('Desktop code, six digits'), findsOneWidget);
    expect(tester.widget<TextField>(field).keyboardType, TextInputType.number);
    semantics.dispose();
  });
}
