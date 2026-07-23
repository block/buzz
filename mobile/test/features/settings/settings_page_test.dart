import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/diagnostics/diagnostics.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  late SharedPreferences preferences;
  late _RecordingCrashReporter reporter;

  Future<DiagnosticsController> createController({
    bool? consent,
    String dsn = 'https://public@example.invalid/1',
  }) async {
    SharedPreferences.setMockInitialValues({
      diagnosticsConsentPreferenceKey: ?consent,
    });
    preferences = await SharedPreferences.getInstance();
    reporter = _RecordingCrashReporter();
    return DiagnosticsController(
      preferences: preferences,
      config: SentryConfig(
        dsn: dsn,
        release: 'buzz@1.2.3',
        dist: '42',
        environment: 'production',
      ),
      crashReporter: reporter,
      log: (_) {},
    );
  }

  Future<void> pumpSettings(
    WidgetTester tester,
    DiagnosticsController controller,
  ) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          diagnosticsControllerProvider.overrideWith((_) => controller),
          savedPrefsProvider.overrideWithValue(preferences),
          relayConfigProvider.overrideWith(() => _TestRelayConfigNotifier()),
        ],
        child: MaterialApp(theme: AppTheme.light(), home: const SettingsPage()),
      ),
    );
    await tester.pump();
  }

  testWidgets('diagnostics switch defaults on with approved copy', (
    tester,
  ) async {
    final controller = await createController();
    await controller.applyStartupConsent();
    await pumpSettings(tester, controller);

    expect(find.text('DIAGNOSTICS'), findsOneWidget);
    expect(find.text('Share crash reports'), findsOneWidget);
    expect(find.text('Sent anonymously to help fix problems.'), findsOneWidget);
    expect(find.text('Share Crash Reports'), findsNothing);
    final toggle = tester.widget<Switch>(find.byType(Switch));
    expect(toggle.value, isTrue);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isNull);
    expect(reporter.initializeCalls, 1);
  });

  testWidgets('explicit opt-in persists and initializes immediately', (
    tester,
  ) async {
    final controller = await createController(consent: false);
    await pumpSettings(tester, controller);

    expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);

    await tester.tap(find.byType(Switch));
    await tester.pumpAndSettle();

    expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
    expect(controller.consentGranted, isTrue);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isTrue);
    expect(reporter.initializeCalls, 1);
  });

  testWidgets('diagnostics switch disables and closes crash reporting', (
    tester,
  ) async {
    final controller = await createController(consent: true);
    await controller.applyStartupConsent();
    await pumpSettings(tester, controller);

    expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);

    await tester.tap(find.byType(Switch));
    await tester.pumpAndSettle();

    expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);
    expect(controller.consentGranted, isFalse);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isFalse);
    expect(reporter.closeCalls, 1);
  });

  testWidgets('reports and rolls back an initialization failure', (
    tester,
  ) async {
    final controller = await createController(consent: false);
    reporter.initializeError = StateError('init failed');
    await pumpSettings(tester, controller);

    await tester.tap(find.byType(Switch));
    await tester.pumpAndSettle();

    expect(
      find.textContaining('Could not update crash reporting:'),
      findsOneWidget,
    );
    expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);
    expect(controller.consentGranted, isFalse);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isFalse);
  });

  testWidgets('existing consent remains visible in an unconfigured build', (
    tester,
  ) async {
    final controller = await createController(consent: true, dsn: '');
    await controller.applyStartupConsent();
    await pumpSettings(tester, controller);

    final toggle = tester.widget<Switch>(find.byType(Switch));
    expect(toggle.value, isTrue);
    expect(toggle.onChanged, isNotNull);

    await tester.tap(find.byType(Switch));
    await tester.pumpAndSettle();

    expect(controller.consentGranted, isFalse);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isFalse);
    expect(reporter.initializeCalls, 0);
  });

  testWidgets('default-on preference is visible without a release DSN', (
    tester,
  ) async {
    final controller = await createController(dsn: '');
    await controller.applyStartupConsent();
    await pumpSettings(tester, controller);

    expect(
      find.text('Crash reporting is unavailable in this build.'),
      findsOneWidget,
    );
    final toggle = tester.widget<Switch>(find.byType(Switch));
    expect(toggle.value, isTrue);
    expect(toggle.onChanged, isNotNull);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isNull);
    expect(reporter.initializeCalls, 0);

    await tester.tap(find.byType(Switch));
    await tester.pumpAndSettle();

    expect(controller.consentGranted, isFalse);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isFalse);
  });

  testWidgets('explicit opt-out is disabled without a release DSN', (
    tester,
  ) async {
    final controller = await createController(consent: false, dsn: '');
    await pumpSettings(tester, controller);

    expect(
      find.text('Crash reporting is unavailable in this build.'),
      findsOneWidget,
    );
    expect(tester.widget<Switch>(find.byType(Switch)).onChanged, isNull);

    await tester.tap(find.byType(Switch), warnIfMissed: false);
    await tester.pumpAndSettle();

    expect(controller.consentGranted, isFalse);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isFalse);
    expect(reporter.initializeCalls, 0);
  });
}

class _TestRelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() =>
      const RelayConfig(baseUrl: 'https://relay.example.invalid');
}

class _RecordingCrashReporter implements CrashReporter {
  int initializeCalls = 0;
  int closeCalls = 0;
  Object? initializeError;

  @override
  Future<void> initialize(SentryConfig config) async {
    initializeCalls += 1;
    if (initializeError case final error?) {
      throw error;
    }
  }

  @override
  Future<void> close() async {
    closeCalls += 1;
  }
}
