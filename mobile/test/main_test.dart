import 'package:buzz/main.dart' as app;
import 'package:buzz/shared/diagnostics/diagnostics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  test('startup continues when crash reporting initialization fails', () async {
    SharedPreferences.setMockInitialValues({
      diagnosticsConsentPreferenceKey: true,
    });
    final preferences = await SharedPreferences.getInstance();
    final logs = <String>[];
    final controller = DiagnosticsController(
      preferences: preferences,
      config: const SentryConfig(
        dsn: 'https://public@example.invalid/1',
        release: 'buzz@1.2.3',
        dist: '42',
        environment: 'production',
      ),
      crashReporter: _FailingCrashReporter(),
    );

    await expectLater(
      app.applyStartupDiagnosticsConsent(controller, log: logs.add),
      completes,
    );

    expect(
      logs.single,
      contains(
        'Diagnostics startup failed; continuing without crash reporting: '
        'Bad state: init failed',
      ),
    );
  });
}

class _FailingCrashReporter implements CrashReporter {
  @override
  Future<void> initialize(SentryConfig config) async {
    throw StateError('init failed');
  }

  @override
  Future<void> close() async {}
}
