import 'dart:async';

import 'package:buzz/shared/diagnostics/diagnostics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  late SharedPreferences preferences;
  late _RecordingCrashReporter reporter;
  late List<String> logs;

  Future<DiagnosticsController> createController({
    Map<String, Object> storedValues = const {},
    String dsn = 'https://public@example.invalid/1',
  }) async {
    SharedPreferences.setMockInitialValues(storedValues);
    preferences = await SharedPreferences.getInstance();
    reporter = _RecordingCrashReporter();
    logs = [];
    return DiagnosticsController(
      preferences: preferences,
      config: SentryConfig(
        dsn: dsn,
        release: 'buzz@1.2.3',
        dist: '42',
        environment: 'production',
      ),
      crashReporter: reporter,
      log: logs.add,
    );
  }

  test('missing preference defaults reporting on for first installs', () async {
    final controller = await createController();

    await controller.applyStartupConsent();

    expect(controller.consentGranted, isTrue);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isNull);
    expect(reporter.initializeCalls, 1);
    expect(reporter.closeCalls, 0);
    expect(
      logs,
      contains('Diagnostics enabled: Sentry initialized after user consent.'),
    );
  });

  test('missing preference defaults reporting on after an upgrade', () async {
    final controller = await createController(
      storedValues: {'existing_preference_from_older_version': true},
    );

    await controller.applyStartupConsent();

    expect(controller.consentGranted, isTrue);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isNull);
    expect(reporter.initializeCalls, 1);
  });

  test('explicitly stored revocation remains off at startup', () async {
    final controller = await createController(
      storedValues: {diagnosticsConsentPreferenceKey: false},
    );

    await controller.applyStartupConsent();

    expect(controller.consentGranted, isFalse);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isFalse);
    expect(reporter.initializeCalls, 0);
    expect(reporter.closeCalls, 0);
    expect(
      logs,
      contains(
        'Diagnostics disabled: user consent is off; Sentry not initialized.',
      ),
    );
  });

  test('empty DSN prevents initialization with default-on reporting', () async {
    final controller = await createController(dsn: '   ');

    await controller.applyStartupConsent();

    expect(controller.consentGranted, isTrue);
    expect(controller.isConfigured, isFalse);
    expect(reporter.initializeCalls, 0);
    expect(
      logs,
      contains(
        'Diagnostics disabled: SENTRY_DSN is empty; Sentry not initialized.',
      ),
    );
  });

  test('cannot opt in to an unconfigured build after revocation', () async {
    final controller = await createController(
      storedValues: {diagnosticsConsentPreferenceKey: false},
      dsn: '   ',
    );

    await expectLater(controller.setConsent(true), throwsA(isA<StateError>()));

    expect(controller.consentGranted, isFalse);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isFalse);
    expect(reporter.initializeCalls, 0);
    expect(
      logs,
      contains(
        'Diagnostics unchanged: SENTRY_DSN is empty; consent not enabled.',
      ),
    );
  });

  test('stored consent initializes once at startup', () async {
    final controller = await createController(
      storedValues: {diagnosticsConsentPreferenceKey: true},
    );

    await controller.applyStartupConsent();
    await controller.applyStartupConsent();

    expect(reporter.initializeCalls, 1);
    expect(reporter.config?.release, 'buzz@1.2.3');
    expect(reporter.config?.dist, '42');
    expect(
      logs,
      contains('Diagnostics already enabled: Sentry initialization skipped.'),
    );
  });

  test('explicit opt-in persists and initializes immediately', () async {
    final controller = await createController(
      storedValues: {diagnosticsConsentPreferenceKey: false},
    );

    await controller.setConsent(true);

    expect(controller.consentGranted, isTrue);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isTrue);
    expect(reporter.initializeCalls, 1);
    expect(
      logs,
      contains('Diagnostics enabled: Sentry initialized after user consent.'),
    );
  });

  test('disabling persists consent and closes immediately', () async {
    final controller = await createController(
      storedValues: {diagnosticsConsentPreferenceKey: true},
    );
    await controller.applyStartupConsent();

    await controller.setConsent(false);

    expect(controller.consentGranted, isFalse);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isFalse);
    expect(reporter.closeCalls, 1);
    expect(
      logs,
      contains('Diagnostics disabled: user consent is off; Sentry closed.'),
    );
  });

  test('can reinitialize after disabling', () async {
    final controller = await createController(
      storedValues: {diagnosticsConsentPreferenceKey: true},
    );
    await controller.applyStartupConsent();

    await controller.setConsent(false);
    await controller.setConsent(true);

    expect(reporter.initializeCalls, 2);
    expect(reporter.closeCalls, 1);
  });

  test('serializes concurrent consent changes', () async {
    final controller = await createController(
      storedValues: {diagnosticsConsentPreferenceKey: false},
    );
    reporter.initializeGate = Completer<void>();

    final enable = controller.setConsent(true);
    await reporter.initializeStarted.future;
    final disable = controller.setConsent(false);

    expect(reporter.closeCalls, 0);
    reporter.initializeGate!.complete();
    await Future.wait([enable, disable]);

    expect(controller.consentGranted, isFalse);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isFalse);
    expect(reporter.initializeCalls, 1);
    expect(reporter.closeCalls, 1);
  });

  test('initialization failure preserves explicit revocation', () async {
    final controller = await createController(
      storedValues: {diagnosticsConsentPreferenceKey: false},
    );
    reporter.initializeError = StateError('init failed');

    await expectLater(controller.setConsent(true), throwsA(isA<StateError>()));

    expect(controller.consentGranted, isFalse);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isFalse);
    expect(reporter.initializeCalls, 1);
  });

  test('close failure preserves revocation and retries teardown', () async {
    final controller = await createController(
      storedValues: {diagnosticsConsentPreferenceKey: true},
    );
    await controller.applyStartupConsent();
    reporter.closeError = StateError('close failed');

    await expectLater(controller.setConsent(false), throwsA(isA<StateError>()));

    expect(controller.consentGranted, isFalse);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isFalse);
    expect(reporter.closeCalls, 1);

    reporter.closeError = null;
    await controller.setConsent(false);

    expect(controller.consentGranted, isFalse);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isFalse);
    expect(reporter.closeCalls, 2);
    expect(
      logs,
      contains('Diagnostics disabled: user consent is off; Sentry closed.'),
    );
  });

  test('continues accepting changes after a failed operation', () async {
    final controller = await createController(
      storedValues: {diagnosticsConsentPreferenceKey: false},
    );
    reporter.initializeError = StateError('init failed');

    await expectLater(controller.setConsent(true), throwsA(isA<StateError>()));
    reporter.initializeError = null;
    await controller.setConsent(true);

    expect(controller.consentGranted, isTrue);
    expect(preferences.getBool(diagnosticsConsentPreferenceKey), isTrue);
    expect(reporter.initializeCalls, 2);
  });

  test('repeating explicit values is idempotent', () async {
    final controller = await createController(
      storedValues: {diagnosticsConsentPreferenceKey: false},
    );

    await controller.setConsent(false);
    await controller.setConsent(true);
    await controller.setConsent(true);

    expect(reporter.initializeCalls, 1);
    expect(reporter.closeCalls, 0);
  });
}

class _RecordingCrashReporter implements CrashReporter {
  int initializeCalls = 0;
  int closeCalls = 0;
  Object? initializeError;
  Object? closeError;
  Completer<void>? initializeGate;
  final initializeStarted = Completer<void>();
  SentryConfig? config;

  @override
  Future<void> initialize(SentryConfig config) async {
    initializeCalls += 1;
    this.config = config;
    if (!initializeStarted.isCompleted) {
      initializeStarted.complete();
    }
    await initializeGate?.future;
    if (initializeError case final error?) {
      throw error;
    }
  }

  @override
  Future<void> close() async {
    closeCalls += 1;
    if (closeError case final error?) {
      throw error;
    }
  }
}
