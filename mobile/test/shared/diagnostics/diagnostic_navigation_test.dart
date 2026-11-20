import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/thread_detail_page.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/features/settings/theme_picker_page.dart';
import 'package:buzz/shared/diagnostics/diagnostic_navigation_observer.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/diagnostics/diagnostic_screens.dart';
import 'package:buzz/shared/hosted/known_community_sync_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  group('the screen a page is named', () {
    final rows = <(Type, DiagnosticScreen)>[
      (HomePage, DiagnosticScreen.channelList),
      (ChannelDetailPage, DiagnosticScreen.channel),
      (ThreadDetailPage, DiagnosticScreen.thread),
      (SettingsPage, DiagnosticScreen.settings),
      (ThemePickerPage, DiagnosticScreen.appearance),
      // Not on the list: never named by its own type.
      (Text, DiagnosticScreen.other),
    ];
    for (final (page, screen) in rows) {
      test('$screen for $page', () {
        expect(DiagnosticScreens.pages[page] ?? DiagnosticScreen.other, screen);
      });
    }
  });

  group('screen breadcrumbs', () {
    const channelId = 'c0ffee-secret-channel-id';
    late _RecordingReporter reporter;

    Future<void> pumpApp(WidgetTester tester, {bool reporting = true}) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      reporter = _RecordingReporter(reporting: reporting);
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            knownCommunitySyncProvider.overrideWithValue(null),
            savedPrefsProvider.overrideWithValue(prefs),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            navigatorObservers: [DiagnosticNavigationObserver(reporter)],
            home: const Scaffold(body: Text('start')),
          ),
        ),
      );
      await tester.pump();
    }

    NavigatorState navigator(WidgetTester tester) =>
        tester.state<NavigatorState>(find.byType(Navigator));

    List<String> trail() => [
      for (final screen in reporter.screens) screen.name,
    ];

    testWidgets('a push and a pop leave the screens the person was on', (
      tester,
    ) async {
      await pumpApp(tester);

      // A route whose settings carry an ID is still named by its page.
      navigator(tester).push(
        MaterialPageRoute<void>(
          settings: const RouteSettings(name: '/channels/$channelId'),
          builder: (_) => const ThemePickerPage(),
        ),
      );
      await tester.pumpAndSettle();
      // An unlisted page is "other", whatever its settings say.
      navigator(tester).push(
        MaterialPageRoute<void>(
          settings: const RouteSettings(name: '/channels/$channelId'),
          builder: (_) => const Scaffold(body: Text('unlisted')),
        ),
      );
      await tester.pumpAndSettle();
      // A dialog is not a screen: opening and closing it leaves nothing.
      final dialog = showDialog<void>(
        context: navigator(tester).context,
        builder: (_) => const AlertDialog(content: Text('dialog')),
      );
      await tester.pumpAndSettle();
      navigator(tester).pop();
      await dialog;
      await tester.pumpAndSettle();
      // Two pops at once leave only the screen they end on.
      navigator(tester).popUntil((route) => route.isFirst);
      await tester.pumpAndSettle();

      expect(trail(), ['other', 'appearance', 'other', 'other']);
      expect(
        reporter.crumbs.map((crumb) => crumb.toString()).join(),
        isNot(contains(channelId)),
      );
    });

    testWidgets('a build without a destination looks at nothing', (
      tester,
    ) async {
      await pumpApp(tester, reporting: false);
      navigator(
        tester,
      ).push(MaterialPageRoute<void>(builder: (_) => const ThemePickerPage()));
      await tester.pumpAndSettle();
      navigator(tester).pop();
      await tester.pumpAndSettle();

      expect(reporter.crumbs, isEmpty);
    });
  });
}

class _RecordingReporter extends NoopDiagnosticReporter {
  _RecordingReporter({required this.reporting});

  @override
  final bool reporting;

  final crumbs = <DiagnosticBreadcrumb>[];

  Iterable<DiagnosticScreen> get screens =>
      crumbs.whereType<ScreenBreadcrumb>().map((crumb) => crumb.screen);

  @override
  void leaveBreadcrumb(DiagnosticBreadcrumb breadcrumb) =>
      crumbs.add(breadcrumb);
}
