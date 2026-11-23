import 'package:buzz/diagnostic_screens.dart';
import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/community/community_membership_provider.dart';
import 'package:buzz/shared/diagnostics/diagnostic_navigation_observer.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  Future<void> pumpSettings(
    WidgetTester tester,
    DiagnosticReporter reporter,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(390, 1400);
    addTearDown(tester.view.reset);
    PackageInfo.setMockInitialValues(
      appName: 'Buzz',
      packageName: 'xyz.block.buzz',
      version: '0.16.0',
      buildNumber: '432',
      buildSignature: '',
    );
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          currentCommunityRoleProvider.overrideWithValue(
            const AsyncData<CommunityMemberRole?>(CommunityMemberRole.member),
          ),
          diagnosticReporterProvider.overrideWithValue(reporter),
          diagnosticScreenClassifierProvider.overrideWithValue(
            diagnosticScreenOf,
          ),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: SettingsPage(
            profileHeader: const SizedBox.shrink(),
            identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  final sendRow = find.byKey(const Key('settings-send-test-report'));
  final crashRow = find.byKey(const Key('settings-test-crash'));

  testWidgets('a build without reporting offers no checks', (tester) async {
    await pumpSettings(tester, const NoopDiagnosticReporter());

    expect(sendRow, findsNothing);
    expect(crashRow, findsNothing);
  });

  testWidgets('a reporting build sends a test report', (tester) async {
    final reporter = _RecordingReporter();
    await pumpSettings(tester, reporter);

    await tester.ensureVisible(sendRow);
    await tester.tap(sendRow);
    await tester.pump();

    expect(reporter.calls, ['sendTestReport']);
    expect(find.text('Test report sent.'), findsOneWidget);
  });

  testWidgets('a test crash happens only once confirmed', (tester) async {
    final reporter = _RecordingReporter();
    await pumpSettings(tester, reporter);

    await tester.ensureVisible(crashRow);
    await tester.tap(crashRow);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(reporter.calls, isEmpty);

    await tester.tap(crashRow);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Crash'));
    await tester.pumpAndSettle();
    expect(reporter.calls, ['crashForTest']);
  });
}

class _RecordingReporter extends NoopDiagnosticReporter {
  final calls = <String>[];

  @override
  bool get reporting => true;

  @override
  void sendTestReport() => calls.add('sendTestReport');

  @override
  void crashForTest() => calls.add('crashForTest');
}
