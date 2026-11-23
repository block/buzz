part of '../settings_page.dart';

/// Checks that a build's diagnostic reporting works end to end. Shown only
/// in builds with a reporting destination, the internal dogfood builds.
class _DiagnosticsSection extends ConsumerWidget {
  const _DiagnosticsSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final reporter = ref.watch(diagnosticReporterProvider);
    return AppListCard(
      dividerIndent: Grid.xs,
      verticalPadding: Grid.twelve,
      children: [
        AppListRow(
          key: const Key('settings-send-test-report'),
          title: 'Send test report',
          subtitle: 'Send a test error to the reporting destination',
          onTap: () {
            reporter.sendTestReport();
            ScaffoldMessenger.of(context).showFeedbackSnackBar(
              const SnackBar(content: Text('Test report sent.')),
              kind: ToastKind.success,
            );
          },
        ),
        AppListRow(
          key: const Key('settings-test-crash'),
          title: 'Test crash',
          subtitle: 'Crash Buzz in native code to check crash reports',
          titleColor: context.colors.error,
          onTap: () async {
            final confirmed = await showDestructiveConfirmation(
              context: context,
              title: 'Crash Buzz?',
              message:
                  'Buzz closes at once. The crash report is sent the next '
                  'time you open it.',
              confirmLabel: 'Crash',
            );
            if (confirmed) reporter.crashForTest();
          },
        ),
      ],
    );
  }
}
