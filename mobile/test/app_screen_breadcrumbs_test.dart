import 'dart:async';

import 'package:buzz/app.dart';
import 'package:buzz/diagnostic_screens.dart';
import 'package:buzz/features/age_gate/age_signal_provider.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/diagnostics/diagnostic_navigation_observer.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  testWidgets('the root screen is named again when auth settles in place', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final auth = Completer<AuthState>();
    final reporter = _RecordingReporter();
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          authProvider.overrideWith(() => _PendingAuthNotifier(auth.future)),
          ageSignalProvider.overrideWith(() => _AllowedAgeSignalNotifier()),
          savedPrefsProvider.overrideWithValue(prefs),
          diagnosticReporterProvider.overrideWithValue(reporter),
          diagnosticScreenClassifierProvider.overrideWithValue(
            diagnosticScreenOf,
          ),
        ],
        child: const App(),
      ),
    );
    await tester.pump();
    // The splash screen is not a listed page.
    expect(reporter.screens, [DiagnosticScreen.other]);

    // Auth swaps the root page's content for onboarding, with no navigation.
    auth.complete(const AuthState(status: AuthStatus.unauthenticated));
    await tester.pump();
    await tester.pump();

    expect(reporter.screens, [
      DiagnosticScreen.other,
      DiagnosticScreen.onboarding,
    ]);
    await tester.pumpWidget(const SizedBox.shrink());
  });
}

class _PendingAuthNotifier extends AuthNotifier {
  _PendingAuthNotifier(this._state);

  final Future<AuthState> _state;

  @override
  Future<AuthState> build() => _state;
}

class _AllowedAgeSignalNotifier extends AgeSignalNotifier {
  @override
  AgeSignalState build() => AgeSignalState.allowed;

  @override
  Future<void> request() async {}
}

class _RecordingReporter extends NoopDiagnosticReporter {
  final screens = <DiagnosticScreen>[];

  @override
  bool get reporting => true;

  @override
  void leaveBreadcrumb(DiagnosticBreadcrumb breadcrumb) {
    if (breadcrumb case ScreenBreadcrumb(:final screen)) screens.add(screen);
  }
}
