import 'package:buzz/diagnostic_screens.dart';
import 'package:buzz/features/pairing/pairing_page.dart';
import 'package:buzz/features/settings/theme_picker_page.dart';
import 'package:buzz/onboarding_app_entry.dart';
import 'package:buzz/shared/diagnostics/diagnostic_navigation_observer.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/widgets/adaptive_conversation_pane.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  group('the screen a page is named', () {
    final rows = <(Widget, DiagnosticScreen?)>[
      (const ThemePickerPage(), DiagnosticScreen.appearance),
      (const OnboardingAppEntry(), DiagnosticScreen.onboarding),
      (
        const OnboardingAppEntry(addingCommunity: true),
        DiagnosticScreen.addCommunity,
      ),
      (const PairingPage(addingCommunity: true), DiagnosticScreen.addCommunity),
      // Send to Desktop recovers an identity; it adds no community.
      (
        const PairingPage(addingCommunity: true, identityRecoveryOnly: true),
        DiagnosticScreen.onboarding,
      ),
      // Not on the list: never named by its own type.
      (const Text('unlisted'), null),
    ];
    for (final (page, screen) in rows) {
      test('$screen for $page', () {
        expect(diagnosticScreenOf(page), screen);
      });
    }
  });

  group('screen breadcrumbs', () {
    const channelId = 'c0ffee-secret-channel-id';
    late _RecordingReporter reporter;
    late GlobalKey<NavigatorState> root;

    Future<void> pumpApp(
      WidgetTester tester, {
      bool reporting = true,
      Widget home = const _Page(DiagnosticScreen.channelList),
    }) async {
      reporter = _RecordingReporter(reporting: reporting);
      root = GlobalKey<NavigatorState>();
      final container = ProviderContainer(
        overrides: [
          diagnosticReporterProvider.overrideWithValue(reporter),
          diagnosticScreenClassifierProvider.overrideWithValue(
            (page) => page is _Page ? page.screen : null,
          ),
        ],
      );
      addTearDown(container.dispose);
      final observer = container.read(diagnosticScreenTrailProvider).observer();
      addTearDown(observer.dispose);
      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: MaterialApp(
            navigatorKey: root,
            navigatorObservers: [observer],
            home: home,
          ),
        ),
      );
      await tester.pumpAndSettle();
    }

    List<String> trail() => [
      for (final screen in reporter.screens) screen.name,
    ];

    testWidgets('a push and a pop leave the screens the person was on', (
      tester,
    ) async {
      await pumpApp(tester);

      // A route whose settings carry an ID is still named by its page.
      root.currentState!.push(
        MaterialPageRoute<void>(
          settings: const RouteSettings(name: '/channels/$channelId'),
          builder: (_) => const _Page(DiagnosticScreen.appearance),
        ),
      );
      await tester.pumpAndSettle();
      // An unlisted page is "other", whatever its settings say.
      root.currentState!.push(
        MaterialPageRoute<void>(
          settings: const RouteSettings(name: '/channels/$channelId'),
          builder: (_) => const Scaffold(body: Text('unlisted')),
        ),
      );
      await tester.pumpAndSettle();
      // A dialog is not a screen: opening and closing it leaves nothing.
      final dialog = showDialog<void>(
        context: root.currentContext!,
        builder: (_) => const AlertDialog(content: Text('dialog')),
      );
      await tester.pumpAndSettle();
      root.currentState!.pop();
      await dialog;
      await tester.pumpAndSettle();
      // Nor is a sheet presented as a page route.
      root.currentState!.push(_SheetRoute());
      await tester.pumpAndSettle();
      root.currentState!.pop();
      await tester.pumpAndSettle();
      // Two pops at once leave only the screen they end on.
      root.currentState!.popUntil((route) => route.isFirst);
      await tester.pumpAndSettle();

      expect(trail(), ['channelList', 'appearance', 'other', 'channelList']);
      expect(
        reporter.crumbs.map((crumb) => crumb.toString()).join(),
        isNot(contains(channelId)),
      );
    });

    testWidgets('a page that leaves from under a sheet names the one shown', (
      tester,
    ) async {
      await pumpApp(tester);

      // Like an Android notification opening a channel over Home's actions
      // sheet: Back leaves the channel, revealing the sheet over Home.
      final sheet = showModalBottomSheet<void>(
        context: root.currentContext!,
        builder: (_) => const Text('actions'),
      );
      await tester.pumpAndSettle();
      root.currentState!.push(
        MaterialPageRoute<void>(
          builder: (_) => const _Page(DiagnosticScreen.channel),
        ),
      );
      await tester.pumpAndSettle();
      root.currentState!.pop();
      await tester.pumpAndSettle();
      root.currentState!.pop();
      await sheet;
      await tester.pumpAndSettle();

      expect(trail(), ['channelList', 'channel', 'channelList']);
    });

    testWidgets('a phone pane names the screen the person is on', (
      tester,
    ) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(390, 844);
      addTearDown(tester.view.reset);
      final controller = ConversationPaneController();
      addTearDown(controller.dispose);
      final search = ConversationPaneController();
      addTearDown(search.dispose);
      await pumpApp(
        tester,
        home: _Page(
          DiagnosticScreen.channelList,
          child: AdaptiveConversationPane(
            controller: controller,
            list: const Text('list'),
          ),
        ),
      );
      NavigatorState pane() =>
          Navigator.of(tester.element(find.text('channel')));

      // The pane's empty first page shows its host, Home, once.
      expect(trail(), ['channelList']);

      controller.open('one', (_) => const _Page(DiagnosticScreen.channel));
      await tester.pumpAndSettle();
      pane().push(
        MaterialPageRoute<void>(
          builder: (_) => const _Page(DiagnosticScreen.thread),
        ),
      );
      await tester.pumpAndSettle();
      Navigator.of(tester.element(find.text('thread'))).pop();
      await tester.pumpAndSettle();

      // A full-screen cover, such as a huddle, closes back onto the open
      // channel rather than the list under it.
      root.currentState!.push(
        MaterialPageRoute<void>(builder: (_) => const Text('huddle')),
      );
      await tester.pumpAndSettle();
      root.currentState!.pop();
      await tester.pumpAndSettle();

      pane().pop();
      await tester.pumpAndSettle();

      // A page that hosts its own pane is named once.
      root.currentState!.push(
        MaterialPageRoute<void>(
          builder: (_) => _Page(
            DiagnosticScreen.search,
            child: AdaptiveConversationPane(
              controller: search,
              list: const Text('results'),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      root.currentState!.pop();
      await tester.pumpAndSettle();

      expect(trail(), [
        'channelList',
        'channel',
        'thread',
        'channel',
        'other',
        'channel',
        'channelList',
        'search',
        'channelList',
      ]);
    });

    testWidgets('a wide section change stays on its screen', (tester) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(1000, 800);
      addTearDown(tester.view.reset);
      final controller = ConversationPaneController();
      addTearDown(controller.dispose);
      await pumpApp(tester);
      root.currentState!.push(
        MaterialPageRoute<void>(
          builder: (_) => _Page(
            DiagnosticScreen.settings,
            child: AdaptiveConversationPane(
              controller: controller,
              persistentSelection: true,
              list: const Text('sections'),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      controller.open('status', (_) => const Text('status'));
      await tester.pumpAndSettle();
      controller.open('profile', (_) => const Text('profile'));
      await tester.pumpAndSettle();
      controller.open(
        'appearance',
        (_) => const _Page(DiagnosticScreen.appearance),
      );
      await tester.pumpAndSettle();

      expect(trail(), ['channelList', 'settings', 'appearance']);
    });

    testWidgets('a build without a destination looks at nothing', (
      tester,
    ) async {
      await pumpApp(tester, reporting: false);
      root.currentState!.push(
        MaterialPageRoute<void>(
          builder: (_) => const _Page(DiagnosticScreen.appearance),
        ),
      );
      await tester.pumpAndSettle();
      root.currentState!.pop();
      await tester.pumpAndSettle();

      expect(reporter.crumbs, isEmpty);
    });
  });
}

/// A page the test's list names [screen].
class _Page extends StatelessWidget {
  const _Page(this.screen, {this.child});

  final DiagnosticScreen screen;
  final Widget? child;

  @override
  Widget build(BuildContext context) =>
      Scaffold(body: child ?? Text(screen.name));
}

class _SheetRoute extends PageRouteBuilder<void>
    with DiagnosticSheetRoute<void> {
  _SheetRoute()
    : super(
        opaque: false,
        pageBuilder: (_, _, _) => const _Page(DiagnosticScreen.channelDetails),
      );
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
